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
        return "Una app".into();
    }
    let path = String::from_utf8_lossy(&buf[..n as usize]).into_owned();
    if let Some(app) = path.split('/').find(|s| s.ends_with(".app")) {
        return app.trim_end_matches(".app").to_string();
    }
    path.rsplit('/').next().unwrap_or("Una app").to_string()
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

