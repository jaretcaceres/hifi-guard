//! Routing policy: keep macOS off the Bluetooth hands-free (call) profile and pick
//! devices by priority — wired/USB twins over Bluetooth, never the Bluetooth mic,
//! highest sample rate available.

use crate::audio::{self, Device};
use crate::macos;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub prefer_wired: bool,
    pub block_bt_mic: bool,
    pub auto_priority: bool,
    pub max_rate: bool,
    pub show_hud: bool,
    pub output_priority: Vec<String>,
    pub input_priority: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            prefer_wired: true,
            block_bt_mic: true,
            auto_priority: true,
            max_rate: true,
            show_hud: true,
            output_priority: Vec::new(),
            input_priority: Vec::new(),
        }
    }
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join("Library/Application Support/HiFiGuard/config.json")
}

impl Config {
    fn load() -> Self {
        std::fs::read(config_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save(&self) {
        let path = config_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, bytes);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Mac,
    Headset,
    Speaker,
    Display,
    Phone,
    Virtual,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Mac => "mac",
            Kind::Headset => "headset",
            Kind::Speaker => "speaker",
            Kind::Display => "display",
            Kind::Phone => "phone",
            Kind::Virtual => "virtual",
        }
    }
}

pub struct Engine {
    pub cfg: Config,
    devices: Vec<Device>,
    output: Option<Device>,
    input: Option<Device>,
    known: HashSet<String>,
    needs_priority: bool,
    bt_mic_apps: Vec<String>,
    last_action: Option<(String, Instant)>,
    fix_count: u32,
    /// Messages for the on-screen HUD, drained by the main loop.
    pub announcements: Vec<(String, Kind)>,
}

impl Engine {
    pub fn new() -> Self {
        let mut engine = Engine {
            cfg: Config::load(),
            devices: Vec::new(),
            output: None,
            input: None,
            known: HashSet::new(),
            needs_priority: true,
            bt_mic_apps: Vec::new(),
            last_action: None,
            fix_count: 0,
            announcements: Vec::new(),
        };
        engine.refresh();
        engine
    }

    // MARK: - State

    pub fn refresh(&mut self) {
        let all = audio::devices();
        let uids: HashSet<String> = all.iter().map(|d| d.uid.clone()).collect();
        if uids != self.known {
            self.known = uids;
            self.needs_priority = true;
        }
        let (out, inp) = (audio::default_device(false), audio::default_device(true));
        self.output = all.iter().find(|d| d.id == out).cloned();
        self.input = all.iter().find(|d| d.id == inp).cloned();
        let mut apps = Vec::new();
        for d in all.iter().filter(|d| d.is_bluetooth() && d.has_input()) {
            for pid in audio::input_clients(d.id) {
                let name = macos::app_name(pid);
                if !apps.contains(&name) {
                    apps.push(name);
                }
            }
        }
        self.bt_mic_apps = apps;
        self.devices = all;
    }

    pub fn bluetooth_ids(&self) -> Vec<audio::ObjectId> {
        self.devices.iter().filter(|d| d.is_bluetooth()).map(|d| d.id).collect()
    }

    pub fn is_call_mode(&self) -> bool {
        self.output.as_ref().is_some_and(Device::is_call_mode) || self.input.as_ref().is_some_and(Device::is_call_mode)
    }

    /// Some app (Discord, Zoom…) picked the Bluetooth mic itself instead of "Default".
    fn app_holds_bt_mic(&self) -> bool {
        !self.bt_mic_apps.is_empty() && !self.input.as_ref().is_some_and(Device::is_bluetooth)
    }

    pub fn tray_state(&self) -> u8 {
        if !self.cfg.enabled { 2 } else if self.is_call_mode() { 1 } else { 0 }
    }

    fn outputs(&self) -> Vec<&Device> {
        self.ranked(self.devices.iter().filter(|d| d.has_output() && d.transport != audio::T_AGGREGATE).collect(), false)
    }

    fn inputs(&self) -> Vec<&Device> {
        self.ranked(self.devices.iter().filter(|d| d.has_input() && d.transport != audio::T_AGGREGATE).collect(), true)
    }

    // MARK: - Commands from the UI / hotkey

    pub fn toggle(&mut self) {
        self.cfg.enabled = !self.cfg.enabled;
        self.cfg.save();
        let msg = if self.cfg.enabled { "HiFi Guard activado" } else { "HiFi Guard desactivado" };
        let kind = self.kind(self.output.as_ref());
        self.announce(msg.into(), kind);
        if self.cfg.enabled {
            self.needs_priority = true;
            self.enforce();
        }
    }

    /// Returns true when the menu should close (Quit).
    pub fn handle(&mut self, msg: &Value) -> bool {
        let uid = msg["uid"].as_str().unwrap_or_default();
        match msg["cmd"].as_str().unwrap_or_default() {
            "toggle" => self.toggle(),
            "set" => {
                let on = msg["value"].as_bool().unwrap_or(false);
                match msg["key"].as_str().unwrap_or_default() {
                    "prefer_wired" => self.cfg.prefer_wired = on,
                    "block_bt_mic" => self.cfg.block_bt_mic = on,
                    "auto_priority" => {
                        self.cfg.auto_priority = on;
                        self.needs_priority = on;
                    }
                    "max_rate" => self.cfg.max_rate = on,
                    "show_hud" => self.cfg.show_hud = on,
                    "launch_at_login" => macos::set_launch_at_login(on),
                    _ => {}
                }
                self.cfg.save();
                self.enforce();
            }
            "selectOutput" => {
                if let Some(d) = self.find(uid) {
                    self.promote(&d, false);
                    if d.is_bluetooth() && self.cfg.prefer_wired && self.output_twin(&d).is_some() {
                        self.cfg.prefer_wired = false;
                    }
                    self.set_output(&d);
                    self.cfg.save();
                }
            }
            "selectInput" => {
                if let Some(d) = self.find(uid) {
                    if d.is_bluetooth() && self.cfg.enabled && self.cfg.block_bt_mic {
                        // Explicitly asking for the headset mic means "I want call mode now".
                        self.cfg.enabled = false;
                    } else {
                        self.promote(&d, true);
                    }
                    audio::set_default(d.id, true);
                    self.cfg.save();
                }
            }
            "mode" => self.set_call_mode(msg["call"].as_bool().unwrap_or(false)),
            "move" => {
                if let Some(d) = self.find(uid) {
                    self.move_device(&d, msg["input"].as_bool().unwrap_or(false), msg["delta"].as_i64().unwrap_or(0));
                }
            }
            "resetPriority" => {
                self.cfg.output_priority.clear();
                self.cfg.input_priority.clear();
                self.cfg.save();
                self.needs_priority = true;
                self.enforce();
            }
            "openApp" => macos::activate_app(msg["name"].as_str().unwrap_or_default()),
            "quit" => return true,
            _ => {}
        }
        self.refresh();
        false
    }

    fn set_call_mode(&mut self, call: bool) {
        if !call {
            self.cfg.enabled = true;
            self.cfg.save();
            self.needs_priority = true;
            self.enforce();
            return;
        }
        self.cfg.enabled = false;
        self.cfg.save();
        let headset = self
            .output
            .as_ref()
            .and_then(|out| self.devices.iter().find(|d| d.is_bluetooth() && d.has_input() && same_family(d, out)))
            .or_else(|| self.devices.iter().find(|d| d.is_bluetooth() && d.has_input()))
            .cloned();
        if let Some(headset) = headset {
            if self.output.as_ref().is_some_and(|o| !o.is_bluetooth()) {
                if let Some(bt_out) = self.devices.iter().find(|d| d.is_bluetooth() && d.has_output() && d.name == headset.name).cloned() {
                    self.set_output(&bt_out);
                }
            }
            audio::set_default(headset.id, true);
        }
    }

    // MARK: - Policy

    pub fn enforce(&mut self) {
        if !self.cfg.enabled {
            return;
        }

        // 1. Something was plugged/unplugged: pick the best device by priority.
        if self.cfg.auto_priority && self.needs_priority {
            self.needs_priority = false;
            if let Some(best) = self.auto_pick(false).filter(|b| Some(b.id) != self.output.as_ref().map(|o| o.id)) {
                self.set_output(&best);
                self.announce(format!("Sonido → {}", best.name), self.kind(Some(&best)));
            }
            self.refresh();
            self.needs_priority = false;
            if let Some(best) = self.auto_pick(true).filter(|b| Some(b.id) != self.input.as_ref().map(|i| i.id)) {
                audio::set_default(best.id, true);
                self.announce(format!("Micro → {}", best.name), self.kind(Some(&best)));
            }
            self.refresh();
            self.needs_priority = false;
        }

        // 2. Bluetooth output with a wired twin (headset base/dongle): use the twin.
        if self.cfg.prefer_wired {
            if let Some(out) = self.output.clone().filter(Device::is_bluetooth) {
                if let Some(twin) = self.output_twin(&out) {
                    self.set_output(&twin);
                    self.announce(format!("Sonido → {} por {}", twin.name, twin.transport_label()), self.kind(Some(&twin)));
                    self.refresh();
                }
            }
        }

        // 3. Never leave the Bluetooth mic as default: that is what triggers call mode.
        if let Some(inp) = self.input.clone().filter(Device::is_bluetooth) {
            if self.cfg.prefer_wired || self.cfg.block_bt_mic {
                let target = self
                    .cfg
                    .prefer_wired
                    .then(|| self.input_twin(&inp))
                    .flatten()
                    .or_else(|| self.cfg.block_bt_mic.then(|| self.fallback_mic()).flatten());
                if let Some(target) = target {
                    if audio::set_default(target.id, true) {
                        self.announce(format!("Modo llamada evitado · micro → {}", target.name), self.kind(Some(&target)));
                    }
                }
                self.refresh();
            }
        }

        // 4. Run wired devices at their best sample rate.
        if self.cfg.max_rate {
            let current: Vec<Device> = [self.output.clone(), self.input.clone()].into_iter().flatten().collect();
            for d in current.iter().filter(|d| !d.is_bluetooth()) {
                if let Some(best) = audio::max_rate(d.id).filter(|&b| b > d.rate) {
                    if audio::set_rate(d.id, best) {
                        self.announce(format!("{} → {}", d.name, audio::format_rate(best)), self.kind(Some(d)));
                    }
                }
            }
            self.refresh();
        }
        self.needs_priority = false;
    }

    fn auto_pick(&self, input: bool) -> Option<Device> {
        let pinned = if input { self.cfg.input_priority.first() } else { self.cfg.output_priority.first() };
        let list = if input { self.inputs() } else { self.outputs() };
        list.into_iter()
            .find(|d| {
                if input && d.is_bluetooth() && self.cfg.block_bt_mic {
                    return false;
                }
                Some(&d.uid) == pinned || (!is_loopback(d) && d.transport != audio::T_VIRTUAL)
            })
            .cloned()
    }

    // MARK: - Ranking

    fn ranked<'a>(&self, mut list: Vec<&'a Device>, input: bool) -> Vec<&'a Device> {
        let order = if input { &self.cfg.input_priority } else { &self.cfg.output_priority };
        let index = |d: &Device| order.iter().position(|u| *u == d.uid).unwrap_or(usize::MAX);
        list.sort_by(|a, b| {
            index(a)
                .cmp(&index(b))
                .then(self.default_rank(a, input).cmp(&self.default_rank(b, input)))
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        list
    }

    fn default_rank(&self, d: &Device, input: bool) -> i32 {
        if is_loopback(d) || d.transport == audio::T_VIRTUAL {
            return 9;
        }
        if input {
            return if d.is_bluetooth() {
                8
            } else if self.looks_like_headset(d) {
                0
            } else if d.transport == audio::T_USB {
                1
            } else if d.transport == audio::T_BUILTIN {
                2
            } else {
                3
            };
        }
        if d.is_bluetooth() {
            if self.cfg.prefer_wired { 1 } else { -1 }
        } else if self.looks_like_headset(d) {
            0
        } else if d.transport == audio::T_USB {
            2
        } else if d.transport == audio::T_BUILTIN {
            3
        } else {
            4
        }
    }

    fn promote(&mut self, d: &Device, input: bool) {
        let mut order: Vec<String> = if input { self.inputs() } else { self.outputs() }.iter().map(|d| d.uid.clone()).collect();
        order.retain(|u| *u != d.uid);
        order.insert(0, d.uid.clone());
        if input { self.cfg.input_priority = order } else { self.cfg.output_priority = order }
    }

    fn move_device(&mut self, d: &Device, input: bool, delta: i64) {
        let mut order: Vec<String> = if input { self.inputs() } else { self.outputs() }.iter().map(|d| d.uid.clone()).collect();
        let Some(from) = order.iter().position(|u| *u == d.uid) else { return };
        let to = (from as i64 + delta).clamp(0, order.len() as i64 - 1) as usize;
        let uid = order.remove(from);
        order.insert(to, uid);
        if input { self.cfg.input_priority = order } else { self.cfg.output_priority = order }
        self.cfg.save();
    }

    // MARK: - Twins & kinds

    fn twins(&self, d: &Device) -> Vec<&Device> {
        self.devices.iter().filter(|o| !o.is_bluetooth() && o.id != d.id && same_family(o, d)).collect()
    }

    fn output_twin(&self, d: &Device) -> Option<Device> {
        self.ranked(self.twins(d).into_iter().filter(|o| o.has_output()).collect(), false).first().map(|d| (*d).clone())
    }

    fn input_twin(&self, d: &Device) -> Option<Device> {
        self.ranked(self.twins(d).into_iter().filter(|o| o.has_input() && !is_loopback(o)).collect(), true)
            .first()
            .map(|d| (*d).clone())
    }

    fn fallback_mic(&self) -> Option<Device> {
        self.inputs()
            .into_iter()
            .find(|d| !d.is_bluetooth() && !is_loopback(d) && d.transport != audio::T_VIRTUAL)
            .cloned()
    }

    fn looks_like_headset(&self, d: &Device) -> bool {
        if self.devices.iter().any(|o| o.is_bluetooth() && o.id != d.id && same_family(o, d)) {
            return true;
        }
        const WORDS: [&str; 18] = [
            "headset", "headphone", "auricular", "casco", "a50", "astro", "arctis", "hyperx", "cloud", "blackshark", "kraken", "g pro",
            "pro x", "virtuoso", "corsair", "game", "chat", "voice",
        ];
        let name = d.name.to_lowercase();
        WORDS.iter().any(|w| name.contains(w))
    }

    pub fn kind(&self, d: Option<&Device>) -> Kind {
        let Some(d) = d else { return Kind::Speaker };
        match d.transport {
            audio::T_BUILTIN => return Kind::Mac,
            audio::T_HDMI | audio::T_DISPLAYPORT => return Kind::Display,
            audio::T_VIRTUAL | audio::T_AGGREGATE => return Kind::Virtual,
            _ => {}
        }
        if d.name.to_lowercase().contains("iphone") {
            Kind::Phone
        } else if d.is_bluetooth() || self.looks_like_headset(d) {
            Kind::Headset
        } else {
            Kind::Speaker
        }
    }

    fn mic_is_headset(&self) -> bool {
        match (&self.input, &self.output) {
            (Some(i), Some(o)) => i.is_bluetooth() || (self.kind(Some(i)) == Kind::Headset && same_family(i, o)),
            _ => false,
        }
    }

    // MARK: - Helpers

    fn find(&self, uid: &str) -> Option<Device> {
        self.devices.iter().find(|d| d.uid == uid).cloned()
    }

    fn set_output(&self, d: &Device) {
        let was_system = self.output.as_ref().is_some_and(|o| o.id == audio::system_output());
        audio::set_default(d.id, false);
        if was_system {
            audio::set_system_output(d.id);
        }
    }

    fn announce(&mut self, message: String, kind: Kind) {
        self.last_action = Some((message.clone(), Instant::now()));
        self.fix_count += 1;
        if self.cfg.show_hud {
            self.announcements.push((message, kind));
        }
    }

    // MARK: - UI state

    fn device_json(&self, d: &Device, input: bool) -> Value {
        let link = link_label(Some(d));
        json!({
            "uid": d.uid,
            "name": d.name,
            "transport": d.transport_label(),
            "link": if d.transport == audio::T_BUILTIN { "Mac".to_string() } else { link },
            "rate": d.rate,
            "rateLabel": audio::format_rate(d.rate),
            "stereo": d.outputs >= 2,
            "bt": d.is_bluetooth(),
            "callMode": d.is_call_mode(),
            "builtin": d.transport == audio::T_BUILTIN,
            "kind": self.kind(Some(d)).as_str(),
            "blocked": input && d.is_bluetooth() && self.cfg.block_bt_mic,
            "btMicOnly": d.is_bluetooth() && d.has_input() && !d.has_output(),
        })
    }

    pub fn state_json(&self) -> Value {
        let last = self.last_action.as_ref().map(|(m, t)| json!({ "text": m, "ago": t.elapsed().as_secs() }));
        json!({
            "enabled": self.cfg.enabled,
            "callMode": self.is_call_mode(),
            "appHoldsBtMic": self.app_holds_bt_mic(),
            "btMicApps": self.bt_mic_apps,
            "hasBtHeadset": self.devices.iter().any(|d| d.is_bluetooth() && d.has_input()),
            "micIsHeadset": self.mic_is_headset(),
            "output": self.output.as_ref().map(|d| self.device_json(d, false)),
            "input": self.input.as_ref().map(|d| self.device_json(d, true)),
            "outputLink": link_label(self.output.as_ref()),
            "outputs": self.outputs().iter().map(|d| self.device_json(d, false)).collect::<Vec<_>>(),
            "inputs": self.inputs().iter().map(|d| self.device_json(d, true)).collect::<Vec<_>>(),
            "lastAction": last,
            "fixCount": self.fix_count,
            "opts": {
                "prefer_wired": self.cfg.prefer_wired,
                "block_bt_mic": self.cfg.block_bt_mic,
                "auto_priority": self.cfg.auto_priority,
                "max_rate": self.cfg.max_rate,
                "show_hud": self.cfg.show_hud,
                "launch_at_login": macos::launch_at_login(),
            },
        })
    }
}

/// Link the device actually uses: USB, A2DP (Bluetooth stereo) or HFP (call mode).
fn link_label(d: Option<&Device>) -> String {
    match d {
        None => "—".into(),
        Some(d) if d.is_bluetooth() => if d.is_call_mode() { "HFP" } else { "A2DP" }.into(),
        Some(d) => d.transport_label().into(),
    }
}

/// "A50" ~ "A50 Game" ~ "A50 Mic": one name prefixes the other, or same first word.
/// A shared brand or owner isn't enough: "Sony WH-1000XM5" isn't "SONY TV" and
/// "Jaret's AirPods" isn't "Jaret's iPhone Microphone".
fn same_family(a: &Device, b: &Device) -> bool {
    if [a, b].iter().any(|d| matches!(d.transport, audio::T_HDMI | audio::T_DISPLAYPORT | audio::T_AIRPLAY)) {
        return false;
    }
    let (x, y) = (a.name.to_lowercase(), b.name.to_lowercase());
    let (short, long) = if x.len() <= y.len() { (&x, &y) } else { (&y, &x) };
    if short.chars().count() < 2 {
        return false;
    }
    if long.starts_with(short.as_str()) {
        return true;
    }
    let first = |s: &str| s.split(' ').next().unwrap_or(s).to_string();
    let owner = first(&x).ends_with("'s") || first(&x).ends_with("’s");
    first(&x) == first(&y) && first(&x).chars().count() >= 2 && !owner
}

/// Stream/monitor mixes show up as inputs but aren't microphones.
fn is_loopback(d: &Device) -> bool {
    let name = d.name.to_lowercase();
    ["stream", "loopback", "monitor"].iter().any(|w| name.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{T_AGGREGATE, T_AIRPLAY, T_BLUETOOTH, T_BUILTIN, T_DISPLAYPORT, T_HDMI, T_USB, T_VIRTUAL};

    fn dev(id: u32, name: &str, transport: u32, inputs: u32, outputs: u32) -> Device {
        Device { id, uid: format!("uid-{id}"), name: name.into(), transport, inputs, outputs, rate: 48_000.0 }
    }

    /// Built by hand: `Engine::new` would read the real config and audio hardware.
    fn engine(devices: Vec<Device>) -> Engine {
        Engine {
            cfg: Config::default(),
            devices,
            output: None,
            input: None,
            known: HashSet::new(),
            needs_priority: false,
            bt_mic_apps: Vec::new(),
            last_action: None,
            fix_count: 0,
            announcements: Vec::new(),
        }
    }

    fn names(list: Vec<&Device>) -> Vec<&str> {
        list.into_iter().map(|d| d.name.as_str()).collect()
    }

    fn a50_setup() -> Vec<Device> {
        vec![
            dev(1, "MacBook Pro Speakers", T_BUILTIN, 0, 2),
            dev(2, "MacBook Pro Microphone", T_BUILTIN, 1, 0),
            dev(3, "A50", T_BLUETOOTH, 1, 2),
            dev(4, "A50 Game", T_USB, 0, 2),
            dev(5, "A50 Mic", T_USB, 1, 0),
        ]
    }

    #[test]
    fn family_matches_prefix_and_shared_first_word() {
        let bt = dev(1, "A50", T_BLUETOOTH, 1, 2);
        assert!(same_family(&bt, &dev(2, "A50 Game", T_USB, 0, 2)));
        assert!(same_family(&dev(2, "A50 Game", T_USB, 0, 2), &dev(3, "a50 mic", T_USB, 1, 0)));
        assert!(!same_family(&bt, &dev(4, "MacBook Pro Speakers", T_BUILTIN, 0, 2)));
    }

    #[test]
    fn family_ignores_too_short_names() {
        assert!(!same_family(&dev(1, "X", T_BLUETOOTH, 1, 2), &dev(2, "X Game", T_USB, 0, 2)));
    }

    #[test]
    fn family_ignores_owner_name_in_device_names() {
        let airpods = dev(1, "Jaret’s AirPods Pro", T_BLUETOOTH, 1, 2);
        assert!(!same_family(&airpods, &dev(2, "Jaret’s iPhone Microphone", 0, 1, 0)));
        assert!(!same_family(&dev(3, "Jaret's AirPods", T_BLUETOOTH, 1, 2), &dev(4, "Jaret's iPad", 0, 1, 0)));
        assert!(same_family(&dev(3, "Jaret's AirPods", T_BLUETOOTH, 1, 2), &dev(5, "Jaret's AirPods Pro", T_USB, 1, 0)));
    }

    #[test]
    fn family_excludes_displays_and_airplay_from_the_same_brand() {
        let headset = dev(1, "Sony WH-1000XM5", T_BLUETOOTH, 1, 2);
        assert!(!same_family(&headset, &dev(2, "SONY TV", T_HDMI, 0, 2)));
        assert!(!same_family(&headset, &dev(3, "Sony Monitor", T_DISPLAYPORT, 0, 2)));
        assert!(!same_family(&headset, &dev(4, "Sony Bravia", T_AIRPLAY, 0, 2)));
    }

    #[test]
    fn twins_route_a_bluetooth_headset_to_its_usb_base() {
        let e = engine(a50_setup());
        let bt = e.devices[2].clone();
        assert_eq!(e.output_twin(&bt).map(|d| d.name), Some("A50 Game".to_string()));
        assert_eq!(e.input_twin(&bt).map(|d| d.name), Some("A50 Mic".to_string()));
    }

    #[test]
    fn input_twin_skips_stream_mixes() {
        let e = engine(vec![dev(1, "A50", T_BLUETOOTH, 1, 2), dev(2, "A50 Stream", T_USB, 2, 0)]);
        assert!(e.input_twin(&e.devices[0]).is_none());
    }

    #[test]
    fn auto_pick_prefers_the_wired_headset() {
        let e = engine(a50_setup());
        assert_eq!(e.auto_pick(false).map(|d| d.name), Some("A50 Game".to_string()));
        assert_eq!(e.auto_pick(true).map(|d| d.name), Some("A50 Mic".to_string()));
    }

    #[test]
    fn auto_pick_never_selects_a_blocked_bluetooth_mic() {
        let e = engine(vec![dev(1, "AirPods Pro", T_BLUETOOTH, 1, 2)]);
        assert!(e.auto_pick(true).is_none());
    }

    #[test]
    fn pinned_bluetooth_mic_is_still_refused_while_blocked() {
        let mut e = engine(vec![dev(1, "AirPods Pro", T_BLUETOOTH, 1, 2), dev(2, "MacBook Pro Microphone", T_BUILTIN, 1, 0)]);
        e.cfg.input_priority = vec!["uid-1".into(), "uid-2".into()];
        assert_eq!(e.auto_pick(true).map(|d| d.name), Some("MacBook Pro Microphone".to_string()));
    }

    #[test]
    fn auto_pick_skips_virtual_devices_unless_pinned() {
        let mut e = engine(vec![dev(1, "BlackHole 2ch", T_VIRTUAL, 2, 2), dev(2, "MacBook Pro Speakers", T_BUILTIN, 0, 2)]);
        assert_eq!(e.auto_pick(false).map(|d| d.name), Some("MacBook Pro Speakers".to_string()));
        e.cfg.output_priority = vec!["uid-1".into()];
        assert_eq!(e.auto_pick(false).map(|d| d.name), Some("BlackHole 2ch".to_string()));
    }

    #[test]
    fn fallback_mic_is_never_bluetooth_virtual_or_loopback() {
        let e = engine(vec![
            dev(1, "AirPods Pro", T_BLUETOOTH, 1, 2),
            dev(2, "BlackHole 2ch", T_VIRTUAL, 2, 2),
            dev(3, "Loopback Audio", T_USB, 2, 0),
            dev(4, "MacBook Pro Microphone", T_BUILTIN, 1, 0),
        ]);
        assert_eq!(e.fallback_mic().map(|d| d.name), Some("MacBook Pro Microphone".to_string()));
    }

    #[test]
    fn user_priority_overrides_default_ranking() {
        let mut e = engine(a50_setup());
        e.cfg.output_priority = vec!["uid-1".into()];
        assert_eq!(names(e.outputs())[0], "MacBook Pro Speakers");
    }

    #[test]
    fn prefer_wired_ranks_bluetooth_below_its_twin() {
        let mut e = engine(a50_setup());
        assert_eq!(names(e.outputs())[..2], ["A50 Game", "A50"]);
        e.cfg.prefer_wired = false;
        assert_eq!(names(e.outputs())[..2], ["A50", "A50 Game"]);
    }

    #[test]
    fn aggregate_devices_are_hidden_from_both_lists() {
        let e = engine(vec![dev(1, "Aggregate", T_AGGREGATE, 2, 2), dev(2, "MacBook Pro Speakers", T_BUILTIN, 0, 2)]);
        assert_eq!(names(e.outputs()), ["MacBook Pro Speakers"]);
        assert!(e.inputs().is_empty());
    }

    #[test]
    fn promote_moves_the_device_to_the_top() {
        let mut e = engine(a50_setup());
        let speakers = e.devices[0].clone();
        e.promote(&speakers, false);
        assert_eq!(e.cfg.output_priority[0], "uid-1");
        assert_eq!(e.cfg.output_priority.iter().filter(|u| *u == "uid-1").count(), 1);
    }

    #[test]
    fn call_mode_means_bluetooth_at_a_low_sample_rate() {
        let mut d = dev(1, "AirPods Pro", T_BLUETOOTH, 1, 2);
        d.rate = 16_000.0;
        assert!(d.is_call_mode());
        d.rate = 44_100.0;
        assert!(!d.is_call_mode());
        let mut usb = dev(2, "USB Mic", T_USB, 1, 0);
        usb.rate = 16_000.0;
        assert!(!usb.is_call_mode());
    }

    #[test]
    fn app_holding_the_mic_is_only_flagged_when_it_isnt_the_default() {
        let bt = dev(1, "AirPods Pro", T_BLUETOOTH, 1, 2);
        let mut e = engine(vec![bt.clone()]);
        e.bt_mic_apps = vec!["Discord".into()];
        e.input = Some(dev(2, "MacBook Pro Microphone", T_BUILTIN, 1, 0));
        assert!(e.app_holds_bt_mic());
        e.input = Some(bt);
        assert!(!e.app_holds_bt_mic());
    }

    #[test]
    fn missing_config_fields_fall_back_to_defaults() {
        let cfg: Config = serde_json::from_str(r#"{ "enabled": false }"#).unwrap();
        assert!(!cfg.enabled);
        assert!(cfg.block_bt_mic && cfg.prefer_wired);
        assert!(cfg.output_priority.is_empty());
    }

    #[test]
    fn link_label_reports_the_real_bluetooth_profile() {
        let mut d = dev(1, "AirPods Pro", T_BLUETOOTH, 1, 2);
        assert_eq!(link_label(Some(&d)), "A2DP");
        d.rate = 16_000.0;
        assert_eq!(link_label(Some(&d)), "HFP");
        assert_eq!(link_label(None), "—");
    }
}
