//! Small AppKit/libSystem helpers: app names for PIDs, the Mac's product image,
//! launch-at-login and a native HUD toast.

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send, sel};
use std::ffi::{c_void, CString};
use std::path::PathBuf;

#[link(name = "UniformTypeIdentifiers", kind = "framework")]
extern "C" {}

extern "C" {
    fn proc_pidpath(pid: i32, buf: *mut u8, size: u32) -> i32;
    fn sysctlbyname(name: *const i8, old: *mut c_void, oldlen: *mut usize, new: *const c_void, newlen: usize) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}
unsafe impl Encode for CGPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for CGSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for CGRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
}
unsafe impl RefEncode for CGRect {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

/// Opaque CGImageRef, typed so Objective-C encodings match.
#[repr(C)]
struct CGImage {
    _private: [u8; 0],
}
unsafe impl RefEncode for CGImage {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGImage", &[]));
}

type Id = *mut AnyObject;

fn ns_string(s: &str) -> Id {
    let c = CString::new(s).unwrap_or_default();
    unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
}

/// "Discord" for /Applications/Discord.app/.../Discord Helper.app/... (helpers record audio for Electron apps).
pub fn app_name(pid: i32) -> String {
    let mut buf = [0u8; 4096];
    let n = unsafe { proc_pidpath(pid, buf.as_mut_ptr(), buf.len() as u32) };
    if n <= 0 {
        return "An app".into();
    }
    let path = String::from_utf8_lossy(&buf[..n as usize]).into_owned();
    if let Some(app) = path.split('/').find(|s| s.ends_with(".app")) {
        return app.trim_end_matches(".app").to_string();
    }
    path.rsplit('/').next().unwrap_or("An app").to_string()
}

pub fn activate_app(name: &str) {
    let _ = std::process::Command::new("/usr/bin/open").args(["-a", name]).spawn();
}

// MARK: - Launch at login (LaunchAgent, like most menu bar utilities)

fn agent_path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Library/LaunchAgents/com.jaret.hifiguard.plist")
}

pub fn launch_at_login() -> bool {
    agent_path().exists()
}

pub fn set_launch_at_login(on: bool) {
    let path = agent_path();
    if !on {
        let _ = std::fs::remove_file(path);
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>com.jaret.hifiguard</string>
	<key>ProgramArguments</key>
	<array>
		<string>{}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>ProcessType</key>
	<string>Interactive</string>
</dict>
</plist>
"#,
        exe.display()
    );
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, plist);
}

// MARK: - This Mac's product image

fn hw_model() -> String {
    let name = CString::new("hw.model").unwrap();
    let mut size = 0usize;
    unsafe {
        sysctlbyname(name.as_ptr(), std::ptr::null_mut(), &mut size, std::ptr::null(), 0);
        let mut buf = vec![0u8; size];
        sysctlbyname(name.as_ptr(), buf.as_mut_ptr() as *mut c_void, &mut size, std::ptr::null(), 0);
        String::from_utf8_lossy(&buf).trim_end_matches('\0').to_string()
    }
}

/// PNG of the Finder icon for this Mac model (e.g. Mac17,9), cached on disk after the first render.
pub fn mac_image_png() -> Option<Vec<u8>> {
    let cache = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Library/Caches/HiFiGuard/mac.png");
    if let Ok(bytes) = std::fs::read(&cache) {
        return Some(bytes);
    }
    let bytes = unsafe {
        let ty: Id = msg_send![class!(UTType), typeWithTag: ns_string(&hw_model()),
                               tagClass: ns_string("com.apple.device-model-code"),
                               conformingToType: std::ptr::null_mut::<AnyObject>()];
        if ty.is_null() {
            return None;
        }
        let workspace: Id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let image: Id = msg_send![workspace, iconForContentType: ty];
        let _: () = msg_send![image, setSize: CGSize { width: 256.0, height: 256.0 }];
        let cg: *mut CGImage = msg_send![image, CGImageForProposedRect: std::ptr::null_mut::<CGRect>(),
                                         context: std::ptr::null_mut::<AnyObject>(), hints: std::ptr::null_mut::<AnyObject>()];
        if cg.is_null() {
            return None;
        }
        let rep: Id = msg_send![class!(NSBitmapImageRep), alloc];
        let rep: Id = msg_send![rep, initWithCGImage: cg];
        let props: Id = msg_send![class!(NSDictionary), dictionary];
        let png: Id = msg_send![rep, representationUsingType: 4usize /* PNG */, properties: props];
        if png.is_null() {
            return None;
        }
        let len: usize = msg_send![png, length];
        let ptr: *const c_void = msg_send![png, bytes];
        let bytes = std::slice::from_raw_parts(ptr as *const u8, len).to_vec();
        let _: () = msg_send![rep, release];
        bytes
    };
    if let Some(dir) = cache.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&cache, &bytes);
    Some(bytes)
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

// MARK: - Liquid Glass

/// Puts the system glass behind a window's content: `NSGlassEffectView` (Liquid Glass,
/// macOS 26+, follows the user's Appearance settings) or the classic popover blur.
/// Returns the class used, for diagnostics.
pub fn apply_glass(ns_window: *mut c_void, radius: f64) -> &'static str {
    unsafe {
        let window = ns_window as Id;
        let content: Id = msg_send![window, contentView];
        if content.is_null() {
            return "none";
        }
        let bounds: CGRect = msg_send![content, bounds];

        // Pin the window and the existing views (the web view) to the system's light/dark mode.
        // Liquid Glass adapts to whatever is behind it and would otherwise flip the page to the
        // opposite scheme (dark text on dark glass).
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let effective: Id = msg_send![app, effectiveAppearance];
        let names: Id = msg_send![class!(NSArray), arrayWithObjects: [ns_string("NSAppearanceNameAqua"), ns_string("NSAppearanceNameDarkAqua")].as_ptr(), count: 2usize];
        let best: Id = msg_send![effective, bestMatchFromAppearancesWithNames: names];
        if !best.is_null() {
            let appearance: Id = msg_send![class!(NSAppearance), appearanceNamed: best];
            let _: () = msg_send![window, setAppearance: appearance];
            let subviews: Id = msg_send![content, subviews];
            let count: usize = msg_send![subviews, count];
            for i in 0..count {
                let v: Id = msg_send![subviews, objectAtIndex: i];
                let _: () = msg_send![v, setAppearance: appearance];
            }
        }

        let glass_class = objc2::runtime::AnyClass::get("NSGlassEffectView");
        let (view, name): (Id, &'static str) = if let Some(cls) = glass_class {
            let v: Id = msg_send![cls, alloc];
            let v: Id = msg_send![v, initWithFrame: bounds];
            let _: () = msg_send![v, setCornerRadius: radius];
            let _: () = msg_send![v, setStyle: 0isize]; // regular: honours the system's clear/tinted setting
            // The glass draws its own edge and depth; a window shadow would trace the square window frame.
            let _: () = msg_send![window, setHasShadow: false];
            (v, "NSGlassEffectView")
        } else {
            let v: Id = msg_send![class!(NSVisualEffectView), alloc];
            let v: Id = msg_send![v, initWithFrame: bounds];
            let _: () = msg_send![v, setMaterial: 6isize]; // popover
            let _: () = msg_send![v, setBlendingMode: 0isize]; // behind window
            let _: () = msg_send![v, setState: 1isize]; // active
            let _: () = msg_send![v, setWantsLayer: true];
            let layer: Id = msg_send![v, layer];
            let _: () = msg_send![layer, setCornerRadius: radius];
            let _: () = msg_send![layer, setMasksToBounds: true];
            (v, "NSVisualEffectView")
        };
        let _: () = msg_send![view, setAutoresizingMask: 18usize]; // width + height sizable
        // Below everything else (the transparent web view sits on top of it).
        let _: () = msg_send![content, addSubview: view, positioned: -1isize, relativeTo: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![view, release];
        // Clip the whole window to the glass shape: nothing (shadow, halo, square corners) renders outside it.
        let _: () = msg_send![content, setWantsLayer: true];
        let clip: Id = msg_send![content, layer];
        let _: () = msg_send![clip, setCornerRadius: radius];
        let _: () = msg_send![clip, setCornerCurve: ns_string("continuous")];
        let _: () = msg_send![clip, setMasksToBounds: true];
        let _: () = msg_send![window, setOpaque: false];
        let clear: Id = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![window, setBackgroundColor: clear];
        name
    }
}

#[link(name = "QuartzCore", kind = "framework")]
extern "C" {}

/// Opening animation done by Core Animation on the whole window (glass included): a short
/// spring from 94% scale plus a fade, like system popovers. Only fades with Reduce Motion on.
/// Returns the layer frame before/after re-anchoring (diagnostics).
pub fn animate_open(ns_window: *mut c_void) -> String {
    unsafe {
        let window = ns_window as Id;
        let _: () = msg_send![window, setAlphaValue: 0.0f64];
        let _: () = msg_send![class!(NSAnimationContext), beginGrouping];
        let context: Id = msg_send![class!(NSAnimationContext), currentContext];
        let _: () = msg_send![context, setDuration: 0.12f64];
        let animator: Id = msg_send![window, animator];
        let _: () = msg_send![animator, setAlphaValue: 1.0f64];
        let _: () = msg_send![class!(NSAnimationContext), endGrouping];

        let workspace: Id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let reduce: bool = msg_send![workspace, accessibilityDisplayShouldReduceMotion];
        let content: Id = msg_send![window, contentView];
        let layer: Id = msg_send![content, layer];
        if reduce || layer.is_null() {
            return "fade only".into();
        }

        // Scale around the centre: move the anchor there and compensate the position so the layer stays put.
        let before: CGRect = msg_send![layer, frame];
        let _: () = msg_send![layer, setAnchorPoint: CGPoint { x: 0.5, y: 0.5 }];
        let moved: CGRect = msg_send![layer, frame];
        let pos: CGPoint = msg_send![layer, position];
        let fixed = CGPoint { x: pos.x + before.origin.x - moved.origin.x, y: pos.y + before.origin.y - moved.origin.y };
        let _: () = msg_send![layer, setPosition: fixed];
        let after: CGRect = msg_send![layer, frame];

        let spring: Id = msg_send![class!(CASpringAnimation), animationWithKeyPath: ns_string("transform.scale")];
        let from: Id = msg_send![class!(NSNumber), numberWithDouble: 0.94f64];
        let to: Id = msg_send![class!(NSNumber), numberWithDouble: 1.0f64];
        let _: () = msg_send![spring, setFromValue: from];
        let _: () = msg_send![spring, setToValue: to];
        let _: () = msg_send![spring, setMass: 1.0f64];
        let _: () = msg_send![spring, setStiffness: 520.0f64];
        let _: () = msg_send![spring, setDamping: 36.0f64];
        let settle: f64 = msg_send![spring, settlingDuration];
        let _: () = msg_send![spring, setDuration: settle];
        let _: () = msg_send![layer, addAnimation: spring, forKey: ns_string("open")];

        format!(
            "before={:?} after={:?} settle={settle:.2}s",
            (before.origin.x, before.origin.y, before.size.width, before.size.height),
            (after.origin.x, after.origin.y, after.size.width, after.size.height)
        )
    }
}

/// Hands keyboard focus to another of our processes (the menu) right after a user click
/// (macOS 14 cooperative activation); the menu then activates itself.
pub fn yield_activation(pid: i32) {
    unsafe {
        let app: Id = msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
        let nsapp: Id = msg_send![class!(NSApplication), sharedApplication];
        let can: bool = msg_send![nsapp, respondsToSelector: sel!(yieldActivationToApplication:)];
        if !app.is_null() && can {
            let _: () = msg_send![nsapp, yieldActivationToApplication: app];
        }
    }
}

pub fn activate_self() {
    unsafe {
        let nsapp: Id = msg_send![class!(NSApplication), sharedApplication];
        let can: bool = msg_send![nsapp, respondsToSelector: sel!(activate)];
        if can {
            let _: () = msg_send![nsapp, activate];
        }
    }
}

/// Calls `f` whenever the user clicks in another app (closes the menu even if macOS
/// declined to make it the active app). Lives for the rest of the process.
pub fn on_click_outside(f: fn()) {
    let block = block2::RcBlock::new(move |_event: *mut AnyObject| f());
    // Left, right and other mouse down.
    let mask: u64 = (1 << 1) | (1 << 3) | (1 << 25);
    unsafe {
        let monitor: Id = msg_send![class!(NSEvent), addGlobalMonitorForEventsMatchingMask: mask, handler: &*block];
        let _ = monitor;
    }
    std::mem::forget(block);
}

/// Class names of a window's content subviews, back to front (debugging aid).
pub fn subview_classes(ns_window: *mut c_void) -> Vec<String> {
    unsafe {
        let content: Id = msg_send![ns_window as Id, contentView];
        let subviews: Id = msg_send![content, subviews];
        let count: usize = msg_send![subviews, count];
        (0..count)
            .map(|i| {
                let v: Id = msg_send![subviews, objectAtIndex: i];
                let f: CGRect = msg_send![v, frame];
                format!("{} {}x{}@{},{}", (*v).class().name(), f.size.width, f.size.height, f.origin.x, f.origin.y)
            })
            .collect()
    }
}

// MARK: - HUD

/// Native toast near the bottom of the screen, like the volume HUD. One panel, reused.
pub struct Hud {
    panel: Id,
    icon: Id,
    title: Id,
}

impl Hud {
    pub fn new() -> Self {
        unsafe {
            let rect = CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: 300.0, height: 60.0 } };
            let panel: Id = msg_send![class!(NSPanel), alloc];
            // Borderless (0) | non-activating panel (1 << 7).
            let panel: Id = msg_send![panel, initWithContentRect: rect, styleMask: 1usize << 7, backing: 2usize, defer: true];
            let _: () = msg_send![panel, setLevel: 25isize]; // status bar level
            let _: () = msg_send![panel, setOpaque: false];
            let clear: Id = msg_send![class!(NSColor), clearColor];
            let _: () = msg_send![panel, setBackgroundColor: clear];
            let _: () = msg_send![panel, setHasShadow: true];
            let _: () = msg_send![panel, setIgnoresMouseEvents: true];
            let _: () = msg_send![panel, setReleasedWhenClosed: false];
            // canJoinAllSpaces | stationary | fullScreenAuxiliary
            let _: () = msg_send![panel, setCollectionBehavior: (1usize << 0) | (1usize << 4) | (1usize << 8)];

            // Labels live in `blur`, a plain container; Liquid Glass (or the HUD blur) sits behind it.
            let blur: Id = msg_send![class!(NSView), alloc];
            let blur: Id = msg_send![blur, initWithFrame: rect];
            let _: () = msg_send![blur, setAutoresizingMask: 18usize];
            if let Some(cls) = objc2::runtime::AnyClass::get("NSGlassEffectView") {
                let glass: Id = msg_send![cls, alloc];
                let glass: Id = msg_send![glass, initWithFrame: rect];
                let _: () = msg_send![glass, setCornerRadius: 30.0f64]; // pill, like the system HUDs
                let _: () = msg_send![glass, setStyle: 0isize];
                let _: () = msg_send![glass, setContentView: blur];
                let _: () = msg_send![panel, setContentView: glass];
            } else {
                let effect: Id = msg_send![class!(NSVisualEffectView), alloc];
                let effect: Id = msg_send![effect, initWithFrame: rect];
                let _: () = msg_send![effect, setMaterial: 13isize]; // HUD window
                let _: () = msg_send![effect, setState: 1isize]; // active
                let _: () = msg_send![effect, setBlendingMode: 0isize]; // behind window
                let _: () = msg_send![effect, setWantsLayer: true];
                let layer: Id = msg_send![effect, layer];
                let _: () = msg_send![layer, setCornerRadius: 18.0f64];
                let _: () = msg_send![layer, setMasksToBounds: true];
                let _: () = msg_send![effect, addSubview: blur];
                let _: () = msg_send![panel, setContentView: effect];
            }

            let icon: Id = msg_send![class!(NSImageView), alloc];
            let icon: Id = msg_send![icon, initWithFrame: CGRect { origin: CGPoint { x: 16.0, y: 14.0 }, size: CGSize { width: 32.0, height: 32.0 } }];
            let white: Id = msg_send![class!(NSColor), labelColor];
            let _: () = msg_send![icon, setContentTintColor: white];
            let _: () = msg_send![blur, addSubview: icon];

            let title: Id = msg_send![class!(NSTextField), labelWithString: ns_string("")];
            let font: Id = msg_send![class!(NSFont), systemFontOfSize: 14.0f64, weight: 0.3f64];
            let _: () = msg_send![title, setFont: font];
            let _: () = msg_send![title, setTextColor: white];
            let _: () = msg_send![blur, addSubview: title];

            let caption: Id = msg_send![class!(NSTextField), labelWithString: ns_string("HiFi Guard")];
            let small: Id = msg_send![class!(NSFont), systemFontOfSize: 11.0f64, weight: 0.23f64];
            let gray: Id = msg_send![class!(NSColor), secondaryLabelColor];
            let _: () = msg_send![caption, setFont: small];
            let _: () = msg_send![caption, setTextColor: gray];
            let _: () = msg_send![caption, setFrameOrigin: CGPoint { x: 60.0, y: 32.0 }];
            let _: () = msg_send![caption, sizeToFit];
            let _: () = msg_send![blur, addSubview: caption];

            Hud { panel, icon, title }
        }
    }

    pub fn show(&self, text: &str, symbol: &str) {
        unsafe {
            let _: () = msg_send![self.title, setStringValue: ns_string(text)];
            let _: () = msg_send![self.title, sizeToFit];
            let _: () = msg_send![self.title, setFrameOrigin: CGPoint { x: 60.0, y: 12.0 }];
            let size: CGSize = msg_send![self.title, fittingSize];

            let image: Id = msg_send![class!(NSImage), imageWithSystemSymbolName: ns_string(symbol), accessibilityDescription: std::ptr::null_mut::<AnyObject>()];
            if !image.is_null() {
                let config: Id = msg_send![class!(NSImageSymbolConfiguration), configurationWithPointSize: 22.0f64, weight: 0.3f64];
                let image: Id = msg_send![image, imageWithSymbolConfiguration: config];
                let _: () = msg_send![self.icon, setImage: image];
            }

            let width = (60.0 + size.width + 22.0).max(220.0);
            let screen: Id = msg_send![class!(NSScreen), mainScreen];
            let visible: CGRect = if screen.is_null() {
                CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: 1440.0, height: 900.0 } }
            } else {
                msg_send![screen, visibleFrame]
            };
            let frame = CGRect {
                origin: CGPoint { x: visible.origin.x + (visible.size.width - width) / 2.0, y: visible.origin.y + 90.0 },
                size: CGSize { width, height: 60.0 },
            };
            let _: () = msg_send![self.panel, setFrame: frame, display: true];
            let visible_now: bool = msg_send![self.panel, isVisible];
            if !visible_now {
                let _: () = msg_send![self.panel, setAlphaValue: 0.0f64];
            }
            let _: () = msg_send![self.panel, orderFrontRegardless];
            let animator: Id = msg_send![self.panel, animator];
            let _: () = msg_send![animator, setAlphaValue: 1.0f64];
        }
    }

    pub fn fade_out(&self) {
        unsafe {
            let animator: Id = msg_send![self.panel, animator];
            let _: () = msg_send![animator, setAlphaValue: 0.0f64];
        }
    }

    pub fn order_out(&self) {
        unsafe {
            let _: () = msg_send![self.panel, orderOut: std::ptr::null_mut::<AnyObject>()];
        }
    }
}
