//! Minimal hand-written bindings to the Core Audio HAL: device snapshots, default
//! device get/set, sample rates, which processes record from a device, and
//! change listeners (so the app never has to poll).

use std::ffi::c_void;
use std::os::raw::c_char;

pub type ObjectId = u32;
type OSStatus = i32;

const fn fourcc(s: &[u8; 4]) -> u32 {
    (s[0] as u32) << 24 | (s[1] as u32) << 16 | (s[2] as u32) << 8 | s[3] as u32
}

pub const SYSTEM: ObjectId = 1;

const SCOPE_GLOBAL: u32 = fourcc(b"glob");
const SCOPE_INPUT: u32 = fourcc(b"inpt");
const SCOPE_OUTPUT: u32 = fourcc(b"outp");

pub const DEVICES: u32 = fourcc(b"dev#");
pub const DEFAULT_INPUT: u32 = fourcc(b"dIn ");
pub const DEFAULT_OUTPUT: u32 = fourcc(b"dOut");
const DEFAULT_SYSTEM_OUTPUT: u32 = fourcc(b"sOut");
pub const PROCESS_LIST: u32 = fourcc(b"prs#");
const DEVICE_UID: u32 = fourcc(b"uid ");
const NAME: u32 = fourcc(b"lnam");
const TRANSPORT: u32 = fourcc(b"tran");
const STREAM_CONFIG: u32 = fourcc(b"slay");
pub const SAMPLE_RATE: u32 = fourcc(b"nsrt");
const AVAILABLE_RATES: u32 = fourcc(b"nsr#");
pub const RUNNING_SOMEWHERE: u32 = fourcc(b"gone");
const PROCESS_PID: u32 = fourcc(b"ppid");
const PROCESS_RUNNING_INPUT: u32 = fourcc(b"piri");
const PROCESS_DEVICES: u32 = fourcc(b"pdv#");

pub const T_BUILTIN: u32 = fourcc(b"bltn");
pub const T_AGGREGATE: u32 = fourcc(b"grup");
pub const T_VIRTUAL: u32 = fourcc(b"virt");
pub const T_USB: u32 = fourcc(b"usb ");
pub const T_BLUETOOTH: u32 = fourcc(b"blue");
pub const T_BLUETOOTH_LE: u32 = fourcc(b"blea");
pub const T_HDMI: u32 = fourcc(b"hdmi");
pub const T_DISPLAYPORT: u32 = fourcc(b"dprt");
pub const T_AIRPLAY: u32 = fourcc(b"airp");

#[repr(C)]
#[derive(Clone, Copy)]
struct Address {
    selector: u32,
    scope: u32,
    element: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ValueRange {
    min: f64,
    max: f64,
}

#[repr(C)]
struct AudioBuffer {
    channels: u32,
    size: u32,
    data: *mut c_void,
}

type Listener = extern "C" fn(ObjectId, u32, *const Address, *mut c_void) -> OSStatus;

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyDataSize(id: ObjectId, addr: *const Address, qs: u32, q: *const c_void, size: *mut u32) -> OSStatus;
    fn AudioObjectGetPropertyData(id: ObjectId, addr: *const Address, qs: u32, q: *const c_void, size: *mut u32, data: *mut c_void) -> OSStatus;
    fn AudioObjectSetPropertyData(id: ObjectId, addr: *const Address, qs: u32, q: *const c_void, size: u32, data: *const c_void) -> OSStatus;
    fn AudioObjectAddPropertyListener(id: ObjectId, addr: *const Address, f: Listener, data: *mut c_void) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringGetCString(s: *const c_void, buf: *mut c_char, size: isize, encoding: u32) -> bool;
    fn CFRelease(obj: *const c_void);
}

fn addr(selector: u32, scope: u32) -> Address {
    Address { selector, scope, element: 0 }
}

fn get<T: Copy + Default>(id: ObjectId, selector: u32) -> Option<T> {
    let a = addr(selector, SCOPE_GLOBAL);
    let mut value = T::default();
    let mut size = std::mem::size_of::<T>() as u32;
    let ok = unsafe { AudioObjectGetPropertyData(id, &a, 0, std::ptr::null(), &mut size, &mut value as *mut T as *mut c_void) };
    (ok == 0).then_some(value)
}

fn set<T: Copy>(id: ObjectId, selector: u32, value: T) -> bool {
    let a = addr(selector, SCOPE_GLOBAL);
    unsafe {
        AudioObjectSetPropertyData(id, &a, 0, std::ptr::null(), std::mem::size_of::<T>() as u32, &value as *const T as *const c_void) == 0
    }
}

fn get_vec<T: Copy + Default>(id: ObjectId, selector: u32, scope: u32) -> Vec<T> {
    let a = addr(selector, scope);
    let mut size = 0u32;
    unsafe {
        if AudioObjectGetPropertyDataSize(id, &a, 0, std::ptr::null(), &mut size) != 0 || size == 0 {
            return Vec::new();
        }
        let mut items = vec![T::default(); size as usize / std::mem::size_of::<T>()];
        if AudioObjectGetPropertyData(id, &a, 0, std::ptr::null(), &mut size, items.as_mut_ptr() as *mut c_void) != 0 {
            return Vec::new();
        }
        items.truncate(size as usize / std::mem::size_of::<T>());
        items
    }
}

fn get_string(id: ObjectId, selector: u32) -> Option<String> {
    let a = addr(selector, SCOPE_GLOBAL);
    let mut cf: *const c_void = std::ptr::null();
    let mut size = std::mem::size_of::<*const c_void>() as u32;
    unsafe {
        if AudioObjectGetPropertyData(id, &a, 0, std::ptr::null(), &mut size, &mut cf as *mut _ as *mut c_void) != 0 || cf.is_null() {
            return None;
        }
        let mut buf = [0 as c_char; 512];
        let ok = CFStringGetCString(cf, buf.as_mut_ptr(), buf.len() as isize, 0x0800_0100 /* UTF-8 */);
        CFRelease(cf);
        ok.then(|| std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
    }
}

fn channels(id: ObjectId, scope: u32) -> u32 {
    let a = addr(STREAM_CONFIG, scope);
    let mut size = 0u32;
    unsafe {
        if AudioObjectGetPropertyDataSize(id, &a, 0, std::ptr::null(), &mut size) != 0 || size < 4 {
            return 0;
        }
        // AudioBufferList: u32 count followed by AudioBuffer[count]; u64 storage keeps it aligned.
        let mut storage = vec![0u64; (size as usize).div_ceil(8)];
        if AudioObjectGetPropertyData(id, &a, 0, std::ptr::null(), &mut size, storage.as_mut_ptr() as *mut c_void) != 0 {
            return 0;
        }
        let base = storage.as_ptr() as *const u8;
        let count = *(base as *const u32);
        let buffers = base.add(std::mem::size_of::<usize>()) as *const AudioBuffer;
        (0..count as usize).map(|i| (*buffers.add(i)).channels).sum()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: ObjectId,
    pub uid: String,
    pub name: String,
    pub transport: u32,
    pub inputs: u32,
    pub outputs: u32,
    pub rate: f64,
}

impl Device {
    pub fn has_input(&self) -> bool { self.inputs > 0 }
    pub fn has_output(&self) -> bool { self.outputs > 0 }
    pub fn is_bluetooth(&self) -> bool { self.transport == T_BLUETOOTH || self.transport == T_BLUETOOTH_LE }
    /// Bluetooth Classic drops to HFP/SCO (8–24 kHz) whenever its mic is open.
    pub fn is_call_mode(&self) -> bool { self.is_bluetooth() && self.rate > 0.0 && self.rate <= 24_000.0 }

    pub fn transport_label(&self) -> &'static str {
        match self.transport {
            T_BLUETOOTH => "Bluetooth",
            T_BLUETOOTH_LE => "Bluetooth LE",
            T_USB => "USB",
            T_BUILTIN => "Integrado",
            T_VIRTUAL => "Virtual",
            T_AGGREGATE => "Agregado",
            T_HDMI | T_DISPLAYPORT => "Pantalla",
            _ => "Otro",
        }
    }
}

pub fn format_rate(rate: f64) -> String {
    if rate <= 0.0 {
        return "—".into();
    }
    let khz = rate / 1000.0;
    if khz.fract() == 0.0 { format!("{} kHz", khz as u32) } else { format!("{:.1} kHz", khz) }
}

pub fn devices() -> Vec<Device> {
    get_vec::<ObjectId>(SYSTEM, DEVICES, SCOPE_GLOBAL)
        .into_iter()
        .filter_map(|id| {
            Some(Device {
                id,
                uid: get_string(id, DEVICE_UID)?,
                name: get_string(id, NAME)?,
                transport: get::<u32>(id, TRANSPORT).unwrap_or(0),
                inputs: channels(id, SCOPE_INPUT),
                outputs: channels(id, SCOPE_OUTPUT),
                rate: get::<f64>(id, SAMPLE_RATE).unwrap_or(0.0),
            })
        })
        .collect()
}

pub fn default_device(input: bool) -> ObjectId {
    get(SYSTEM, if input { DEFAULT_INPUT } else { DEFAULT_OUTPUT }).unwrap_or(0)
}

pub fn system_output() -> ObjectId {
    get(SYSTEM, DEFAULT_SYSTEM_OUTPUT).unwrap_or(0)
}

pub fn set_default(id: ObjectId, input: bool) -> bool {
    set(SYSTEM, if input { DEFAULT_INPUT } else { DEFAULT_OUTPUT }, id)
}

pub fn set_system_output(id: ObjectId) -> bool {
    set(SYSTEM, DEFAULT_SYSTEM_OUTPUT, id)
}

/// Highest nominal sample rate the device supports (capped at 192 kHz).
pub fn max_rate(id: ObjectId) -> Option<f64> {
    get_vec::<ValueRange>(id, AVAILABLE_RATES, SCOPE_GLOBAL)
        .iter()
        .map(|r| r.max.min(192_000.0))
        .fold(None, |best, r| Some(best.map_or(r, |b: f64| b.max(r))))
}

pub fn set_rate(id: ObjectId, rate: f64) -> bool {
    set(id, SAMPLE_RATE, rate)
}

/// PIDs of processes currently recording from `device`.
pub fn input_clients(device: ObjectId) -> Vec<i32> {
    get_vec::<ObjectId>(SYSTEM, PROCESS_LIST, SCOPE_GLOBAL)
        .into_iter()
        .filter(|&p| get::<u32>(p, PROCESS_RUNNING_INPUT) == Some(1))
        .filter(|&p| get_vec::<ObjectId>(p, PROCESS_DEVICES, SCOPE_INPUT).contains(&device))
        .filter_map(|p| get::<i32>(p, PROCESS_PID))
        .collect()
}

/// Calls `callback` (on a Core Audio thread) whenever `selector` changes on `object`.
pub fn listen(object: ObjectId, selector: u32, callback: extern "C" fn()) {
    extern "C" fn trampoline(_: ObjectId, _: u32, _: *const Address, data: *mut c_void) -> OSStatus {
        let f: extern "C" fn() = unsafe { std::mem::transmute(data) };
        f();
        0
    }
    let a = addr(selector, SCOPE_GLOBAL);
    unsafe { AudioObjectAddPropertyListener(object, &a, trampoline, callback as *mut c_void) };
}
