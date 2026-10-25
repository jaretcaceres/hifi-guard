//! HiFi Guard: menu bar app that keeps Bluetooth headsets out of call mode.
//!
//! Idle cost is ~0: the event loop sleeps until Core Audio, the tray icon or the
//! hotkey wakes it. The menu is a separate short-lived process (see `menu.rs`), so
//! WebKit only exists while the menu is open.

mod audio;
mod engine;
mod macos;
mod menu;

use engine::{Engine, Kind};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const DEBOUNCE: Duration = Duration::from_millis(250);
const HUD_TIME: Duration = Duration::from_millis(2200);
/// How long a pre-rendered menu survives after the pointer leaves the icon without clicking.
const PREWARM_TIME: Duration = Duration::from_secs(2);

enum UserEvent {
    Audio,
    Tray(TrayIconEvent),
    HotKey,
    Ipc(String),
    MenuClosed(u32),
}

static PROXY: OnceLock<Mutex<EventLoopProxy<UserEvent>>> = OnceLock::new();

fn send(event: UserEvent) {
    if let Some(proxy) = PROXY.get() {
        let _ = proxy.lock().map(|p| p.send_event(event));
    }
}

extern "C" fn on_audio_change() {
    send(UserEvent::Audio);
}

/// A running `hifiguard --menu` child process, either pre-rendered and hidden or on screen.
struct MenuProcess {
    child: Child,
    stdin: ChildStdin,
    visible: bool,
}

impl MenuProcess {
    fn spawn(rect: &tray_icon::Rect, state: &str, hidden: bool) -> Option<Self> {
        let r = rect;
        let mut command = Command::new(std::env::current_exe().ok()?);
        command
            .arg("--menu")
            .args([r.position.x, r.position.y, r.size.width as f64, r.size.height as f64].map(|v| v.to_string()));
        if hidden {
            command.arg("--hidden");
        }
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
        let mut stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        writeln!(stdin, "{state}").ok()?;
        let pid = child.id();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                send(UserEvent::Ipc(line));
            }
            send(UserEvent::MenuClosed(pid));
        });
        Some(MenuProcess { child, stdin, visible: !hidden })
    }

    fn show(&mut self) {
        self.visible = true;
        // We just got the user's click, so we may pass keyboard focus on to the menu.
        macos::yield_activation(self.child.id() as i32);
        let _ = writeln!(self.stdin, "show");
    }

    fn update(&mut self, state: &str) {
        let _ = writeln!(self.stdin, "{state}");
    }
}

impl Drop for MenuProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tray_icon(state: u8) -> Icon {
    let bytes: &[u8] = match state {
        1 => include_bytes!("../assets/tray-warn.rgba"),
        2 => include_bytes!("../assets/tray-off.rgba"),
        _ => include_bytes!("../assets/tray-ok.rgba"),
    };
    Icon::from_rgba(bytes.to_vec(), 36, 36).expect("tray icon")
}

fn symbol(kind: Kind) -> &'static str {
    match kind {
        Kind::Mac => "laptopcomputer",
        Kind::Headset => "headphones",
        Kind::Speaker => "hifispeaker.fill",
        Kind::Display => "display",
        Kind::Phone => "iphone",
        Kind::Virtual => "waveform",
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--menu") {
        return menu::run(&args[2..]);
    }

    let mut engine = Engine::new();
    engine.enforce();
    if args.iter().any(|a| a == "--state") {
        // Dev aid: print what the menu would render (used for UI previews).
        println!("{}", engine.state_json());
        return;
    }

    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    let _ = PROXY.set(Mutex::new(event_loop.create_proxy()));

    let mut tray: Option<TrayIcon> = None;
    let mut tray_state = u8::MAX;
    let mut hotkeys: Option<GlobalHotKeyManager> = None;
    let mut hud: Option<macos::Hud> = None;
    let mut menu: Option<MenuProcess> = None;
    let mut listened: HashSet<audio::ObjectId> = HashSet::new();

    let mut refresh_at: Option<Instant> = None;
    let mut hud_fade_at: Option<Instant> = None;
    let mut hud_out_at: Option<Instant> = None;
    let mut closed_at = Instant::now() - Duration::from_secs(10);
    let mut prewarm_until: Option<Instant> = None;

    event_loop.run(move |event, _, control_flow| {
        let mut changed = false;

        match event {
            Event::NewEvents(StartCause::Init) => {
                for selector in [audio::DEVICES, audio::DEFAULT_INPUT, audio::DEFAULT_OUTPUT, audio::PROCESS_LIST] {
                    audio::listen(audio::SYSTEM, selector, on_audio_change);
                }
                TrayIconEvent::set_event_handler(Some(|e| send(UserEvent::Tray(e))));
                GlobalHotKeyEvent::set_event_handler(Some(|e: GlobalHotKeyEvent| {
                    if e.state == HotKeyState::Pressed {
                        send(UserEvent::HotKey);
                    }
                }));
                if let Ok(manager) = GlobalHotKeyManager::new() {
                    let _ = manager.register(HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER), Code::KeyH));
                    hotkeys = Some(manager);
                }
                tray = TrayIconBuilder::new()
                    .with_icon(tray_icon(0))
                    .with_icon_as_template(true)
                    .with_tooltip("HiFi Guard")
                    .build()
                    .ok();
                hud = Some(macos::Hud::new());
                refresh_at = Some(Instant::now());
                // `--open` shows the menu right away and `--hud` a sample toast (testing, screenshots).
                if args.iter().any(|a| a == "--open") {
                    if let Some(rect) = tray.as_ref().and_then(TrayIcon::rect) {
                        menu = MenuProcess::spawn(&rect, &engine.state_json().to_string(), false);
                    }
                }
                if args.iter().any(|a| a == "--hud") {
                    engine.announcements.push(("Modo llamada evitado · micro → A50 Mic".into(), Kind::Headset));
                }
            }

            Event::UserEvent(UserEvent::Audio) => refresh_at = Some(Instant::now() + DEBOUNCE),

            Event::UserEvent(UserEvent::HotKey) => {
                engine.toggle();
                engine.refresh();
                changed = true;
            }

            // Pointer over the icon: render the menu off screen so the click shows it instantly.
            Event::UserEvent(UserEvent::Tray(TrayIconEvent::Enter { rect, .. })) => {
                prewarm_until = None;
                if menu.is_none() && closed_at.elapsed() > Duration::from_millis(400) {
                    menu = MenuProcess::spawn(&rect, &engine.state_json().to_string(), true);
                }
            }

            Event::UserEvent(UserEvent::Tray(TrayIconEvent::Leave { .. })) => {
                if menu.as_ref().is_some_and(|m| !m.visible) {
                    prewarm_until = Some(Instant::now() + PREWARM_TIME);
                }
            }

            Event::UserEvent(UserEvent::Tray(TrayIconEvent::Click {
                rect, button: MouseButton::Left, button_state: MouseButtonState::Up, ..
            })) => {
                prewarm_until = None;
                match &mut menu {
                    Some(m) if m.visible => {
                        menu = None;
                        closed_at = Instant::now();
                    }
                    Some(m) => m.show(),
                    // No pre-rendered menu (e.g. keyboard navigation of the menu bar), unless this
                    // click just closed it by taking its focus.
                    None if closed_at.elapsed() > Duration::from_millis(300) => {
                        menu = MenuProcess::spawn(&rect, &engine.state_json().to_string(), false);
                    }
                    None => {}
                }
            }

            Event::UserEvent(UserEvent::MenuClosed(pid)) => {
                if menu.as_ref().is_some_and(|m| m.child.id() == pid) {
                    menu = None;
                    closed_at = Instant::now();
                }
            }

            Event::UserEvent(UserEvent::Ipc(body)) => {
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&body) {
                    match msg["cmd"].as_str() {
                        Some("close") => {
                            menu = None;
                            closed_at = Instant::now();
                        }
                        Some("ready") => changed = true,
                        _ => {
                            if engine.handle(&msg) {
                                menu = None;
                                *control_flow = ControlFlow::Exit;
                                return;
                            }
                            changed = true;
                        }
                    }
                }
            }

            _ => {}
        }

        let now = Instant::now();
        if refresh_at.is_some_and(|t| now >= t) {
            refresh_at = None;
            engine.refresh();
            engine.enforce();
            // Profile switches (A2DP <-> HFP) and apps grabbing the Bluetooth mic only show up per device.
            for id in engine.bluetooth_ids() {
                if listened.insert(id) {
                    audio::listen(id, audio::SAMPLE_RATE, on_audio_change);
                    audio::listen(id, audio::RUNNING_SOMEWHERE, on_audio_change);
                }
            }
            changed = true;
        }
        if prewarm_until.is_some_and(|t| now >= t) {
            prewarm_until = None;
            if menu.as_ref().is_some_and(|m| !m.visible) {
                menu = None;
            }
        }
        if hud_fade_at.is_some_and(|t| now >= t) {
            hud_fade_at = None;
            hud_out_at = Some(now + Duration::from_millis(400));
            if let Some(h) = &hud {
                h.fade_out();
            }
        }
        if hud_out_at.is_some_and(|t| now >= t) {
            hud_out_at = None;
            if let Some(h) = &hud {
                h.order_out();
            }
        }

        if changed {
            if let Some((text, kind)) = engine.announcements.drain(..).last() {
                if let Some(h) = &hud {
                    h.show(&text, symbol(kind));
                    hud_fade_at = Some(now + HUD_TIME);
                    hud_out_at = None;
                }
            }
            let state = engine.tray_state();
            if state != tray_state {
                tray_state = state;
                if let Some(t) = &tray {
                    let _ = t.set_icon_with_as_template(Some(tray_icon(state)), true);
                }
            }
            if let Some(m) = &mut menu {
                m.update(&engine.state_json().to_string());
            }
        }

        // Sleep until the next deadline, or until something wakes us.
        let _ = &hotkeys;
        *control_flow = match [refresh_at, hud_fade_at, hud_out_at, prewarm_until].into_iter().flatten().min() {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        };
    });
}
