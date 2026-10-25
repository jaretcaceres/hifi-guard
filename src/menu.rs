//! The menu runs in its own short-lived process (`hifiguard --menu x y w h [--hidden]`).
//! When it closes the process exits, and WebKit's helper processes (web content,
//! GPU, networking) go with it, so the resident app stays at its idle footprint.
//!
//! With `--hidden` it is pre-rendered while the pointer is over the menu bar icon and
//! appears instantly on `show`.
//!
//! Protocol: the parent writes one JSON state per line (or `show`) to stdin; the menu
//! writes one JSON command per line to stdout. EOF on stdin closes the menu.

use std::io::{BufRead, Write};
use std::sync::{Mutex, OnceLock};
use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS, WindowBuilderExtMacOS, WindowExtMacOS};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

pub const WIDTH: f64 = 380.0;
pub const HEIGHT: f64 = 530.0;
const RADIUS: f64 = 22.0;

enum MenuEvent {
    State(String),
    Show,
    Ready,
    Exit,
}

static PROXY: OnceLock<Mutex<EventLoopProxy<MenuEvent>>> = OnceLock::new();

fn send(event: MenuEvent) {
    if let Some(proxy) = PROXY.get() {
        let _ = proxy.lock().map(|p| p.send_event(event));
    }
}

fn close() {
    send(MenuEvent::Exit);
}

fn emit(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// `args`: tray icon rect in physical pixels — x, y, width, height — and optionally `--hidden`.
pub fn run(args: &[String]) {
    let hidden = args.iter().any(|a| a == "--hidden");
    // Dev aids for screenshots: keep the menu up without focus, and open on a given tab.
    let pinned = std::env::var_os("HIFIGUARD_PIN").is_some();
    let tab: u8 = std::env::var("HIFIGUARD_TAB").ok().and_then(|t| t.parse().ok()).unwrap_or(0);
    let t0 = std::time::Instant::now();
    let debug_t = std::env::var_os("HIFIGUARD_DEBUG").is_some();
    let mark = move |what: &str| {
        if debug_t {
            eprintln!("[{:>5} ms] {what}", t0.elapsed().as_millis());
        }
    };
    let rect: Vec<f64> = args.iter().filter_map(|a| a.parse().ok()).collect();
    let [x, y, w, h] = rect[..] else { return };

    let mut first = String::new();
    if std::io::stdin().lock().read_line(&mut first).unwrap_or(0) == 0 {
        return;
    }

    let mut event_loop = EventLoopBuilder::<MenuEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    let _ = PROXY.set(Mutex::new(event_loop.create_proxy()));

    std::thread::spawn(|| {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            send(if line == "show" { MenuEvent::Show } else { MenuEvent::State(line) });
        }
        send(MenuEvent::Exit);
    });

    let window = WindowBuilder::new()
        .with_title("HiFi Guard")
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_visible(false)
        .with_always_on_top(true)
        .with_has_shadow(true)
        .with_inner_size(LogicalSize::new(WIDTH, HEIGHT))
        .build(&event_loop)
        .expect("menu window");

    // Centre under the menu bar icon, kept on screen.
    let scale = window.scale_factor();
    let width = WIDTH * scale;
    let mut left = x + w / 2.0 - width / 2.0;
    let mut top = y + h + 6.0 * scale;
    if let Some(monitor) = window.current_monitor() {
        let min = monitor.position().x as f64 + 8.0 * scale;
        let max = monitor.position().x as f64 + monitor.size().width as f64 - width - 8.0 * scale;
        if h <= 0.0 {
            // No laid-out status item yet (e.g. `--open` at launch): top-right corner.
            left = max;
            top = monitor.position().y as f64 + 40.0 * scale;
        }
        left = left.clamp(min, max.max(min));
    }
    window.set_outer_position(PhysicalPosition::new(left, top));

    mark("window created");
    let mac = crate::macos::mac_image_png()
        .map(|png| format!("data:image/png;base64,{}", crate::macos::base64(&png)))
        .unwrap_or_default();
    mark("mac image loaded");
    let init = format!("window.__STATE__ = {}; window.__MAC__ = {:?}; window.__TAB__ = {tab};", first.trim(), mac);
    let webview = WebViewBuilder::new()
        .with_transparent(true)
        .with_initialization_script(&init)
        .with_html(include_str!("../ui/index.html"))
        .with_ipc_handler(|req| {
            // Show the window only once the page has painted, so it never flashes empty.
            if req.body().contains("\"ready\"") {
                send(MenuEvent::Ready);
            }
            emit(req.body());
        })
        .build(&window)
        .expect("menu webview");
    mark("webview built");
    // Liquid Glass behind the (transparent) web view; added after it so it can be placed underneath.
    let material = crate::macos::apply_glass(window.ns_window(), RADIUS);
    if std::env::var_os("HIFIGUARD_DEBUG").is_some() {
        eprintln!("material={material} subviews={:?}", crate::macos::subview_classes(window.ns_window()));
    }

    let debug = std::env::var_os("HIFIGUARD_DEBUG").is_some();
    let soon = || std::time::Instant::now() + std::time::Duration::from_millis(600);
    let mut ready = false;
    // Safety net: once showing is wanted, show by this time even if the page never reports 'ready'.
    let mut show_by = (!hidden).then(soon);
    let mut shown = false;

    event_loop.run(move |event, _, control_flow| {
        match event {
            Event::UserEvent(MenuEvent::Ready) => ready = true,
            Event::UserEvent(MenuEvent::Show) => {
                show_by.get_or_insert_with(soon);
            }
            Event::UserEvent(MenuEvent::State(state)) => {
                let _ = webview.evaluate_script(&format!("window.render({state})"));
            }
            Event::UserEvent(MenuEvent::Exit) => *control_flow = ControlFlow::Exit,
            Event::WindowEvent { event: WindowEvent::Focused(false), .. } if shown && !pinned => *control_flow = ControlFlow::Exit,
            _ => {}
        }
        if *control_flow == ControlFlow::Exit {
            return;
        }

        if let (false, Some(by)) = (shown, show_by) {
            if ready || std::time::Instant::now() >= by {
                shown = true;
                window.set_visible(true);
                crate::macos::activate_self();
                window.set_focus();
                let anim = crate::macos::animate_open(window.ns_window());
                if !pinned {
                    crate::macos::on_click_outside(close);
                }
                mark("shown");
                if debug {
                    eprintln!("shown ({}); open animation: {anim}", if ready { "ready" } else { "timeout" });
                }
            }
        }
        *control_flow = match (shown, show_by) {
            (false, Some(by)) => ControlFlow::WaitUntil(by),
            _ => ControlFlow::Wait,
        };
    });
}
