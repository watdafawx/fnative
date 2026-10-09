//! For fse plugins written in Rust: the host table of include/fse.h and safe wrappers around it.
//!
//! ```ignore
//! #[no_mangle]
//! pub unsafe extern "C" fn fse_plugin_init(host: *const fse_plugin::Host) -> i32 {
//!     fse_plugin::init(host, "myplugin");
//!     fse_plugin::register("myplugin", "hello", hello, fse_plugin::THREADSAFE);
//!     0
//! }
//! fse_plugin::export!(hello, |input: &str| -> Result<String, String> { Ok(input.to_uppercase()) });
//! ```

use std::cell::RefCell;
use std::ffi::{c_char, c_void, CStr, CString};

pub const ABI: u32 = 1;
pub const THREADSAFE: u32 = 1;

pub type PluginFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, usize, *mut *const c_char,
                                         *mut usize) -> i32;

#[repr(C)]
pub struct Host {
    pub abi: u32,
    pub version: *const c_char,
    pub log: unsafe extern "C" fn(*const c_char, *const c_char),
    pub engine_symbol: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    pub register_fn: unsafe extern "C" fn(*const c_char, *const c_char, PluginFn, *mut c_void, u32) -> i32,
    // core 0.3.0
    pub build: *const c_char,
    pub field_offset: unsafe extern "C" fn(*const c_char, *const c_char) -> i64,
    pub class_size: unsafe extern "C" fn(*const c_char) -> i64,
    // core 0.4.0
    pub call: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, usize, *mut *const c_char,
                                   *mut usize) -> i32,
    // core 0.7.0
    pub emit: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, usize),
    pub read: unsafe extern "C" fn(u64, *const c_char, *const c_char, u32, *mut *const c_char, *mut usize) -> i32,
    // core 0.8.0
    pub emit_local: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, usize),
    // core 0.10.0
    pub scenario: unsafe extern "C" fn() -> u64,
}

static mut HOST: *const Host = std::ptr::null();
static mut NAME: Option<CString> = None;

fn c(s: &str) -> CString {
    CString::new(s.replace('\0', " ")).unwrap_or_default()
}

/// remember the host (call first in fse_plugin_init)
pub unsafe fn init(host: *const Host, plugin: &str) -> bool {
    if host.is_null() || (*host).abi != ABI {
        return false;
    }
    HOST = host;
    NAME = Some(c(plugin));
    true
}

#[allow(static_mut_refs)]
pub fn host() -> Option<&'static Host> {
    unsafe { HOST.as_ref() }
}

/// the core's version as numbers, e.g. (0, 4, 0)
pub fn core_version() -> (u32, u32, u32) {
    let Some(h) = host() else { return (0, 0, 0) };
    let v = unsafe { CStr::from_ptr(h.version) }.to_string_lossy().into_owned();
    let mut n = v.split('.').map(|x| x.parse().unwrap_or(0));
    (n.next().unwrap_or(0), n.next().unwrap_or(0), n.next().unwrap_or(0))
}

#[allow(static_mut_refs)]
pub fn log(msg: &str) {
    if let Some(h) = host() {
        let name = unsafe { NAME.as_ref().map(|n| n.as_ptr()).unwrap_or(c"?".as_ptr()) };
        let m = c(msg);
        unsafe { (h.log)(name, m.as_ptr()) }
    }
}

pub fn register(plugin: &str, name: &str, f: PluginFn, flags: u32) -> bool {
    let Some(h) = host() else { return false };
    let (p, n) = (c(plugin), c(name));
    unsafe { (h.register_fn)(p.as_ptr(), n.as_ptr(), f, std::ptr::null_mut(), flags) == 0 }
}

pub fn engine_symbol(name: &str) -> Option<usize> {
    let h = host()?;
    let n = c(name);
    let p = unsafe { (h.engine_symbol)(n.as_ptr()) };
    (!p.is_null()).then_some(p as usize)
}

pub fn build() -> String {
    host().filter(|_| core_version() >= (0, 3, 0))
        .map(|h| unsafe { CStr::from_ptr(h.build) }.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn field_offset(class: &str, field: &str) -> Option<u64> {
    let h = host().filter(|_| core_version() >= (0, 3, 0))?;
    let (a, b) = (c(class), c(field));
    let o = unsafe { (h.field_offset)(a.as_ptr(), b.as_ptr()) };
    (o >= 0).then_some(o as u64)
}

/// another plugin's function (any registered one; mind threadsafety: from a worker thread call only threadsafe ones)
pub fn call(plugin: &str, name: &str, input: &[u8]) -> Result<Vec<u8>, String> {
    let h = host().filter(|_| core_version() >= (0, 4, 0)).ok_or("the core is older than 0.4.0: no host call")?;
    let (p, n) = (c(plugin), c(name));
    let mut out: *const c_char = std::ptr::null();
    let mut len = 0usize;
    let rc = unsafe { (h.call)(p.as_ptr(), n.as_ptr(), input.as_ptr() as *const c_char, input.len(), &mut out, &mut len) };
    let bytes = if out.is_null() { Vec::new() } else { unsafe { std::slice::from_raw_parts(out as *const u8, len) }.to_vec() };
    if rc == 0 { Ok(bytes) } else { Err(String::from_utf8_lossy(&bytes).into_owned()) }
}

/// an event for Lua (native.events): `data` is JSON by convention. Any thread; cheap (a lock and a copy). Emitted
/// while the simulation updates, it is also part of that tick's "fse-event" on every peer (see emit_local).
#[allow(static_mut_refs)]
pub fn emit(name: &str, data: &str) {
    let Some(h) = host().filter(|_| core_version() >= (0, 7, 0)) else { return };
    let plugin = unsafe { NAME.as_ref().map(|n| n.as_ptr()).unwrap_or(c"?".as_ptr()) };
    let n = c(name);
    unsafe { (h.emit)(plugin, n.as_ptr(), data.as_ptr() as *const c_char, data.len()) }
}

/// an event that is never part of the simulation (from the render thread, the mouse, a worker of your own): it goes
/// to native.events only, never to the "fse-event" every peer gets
#[allow(static_mut_refs)]
pub fn emit_local(name: &str, data: &str) {
    let Some(h) = host().filter(|_| core_version() >= (0, 8, 0)) else { return emit(name, data) };
    let plugin = unsafe { NAME.as_ref().map(|n| n.as_ptr()).unwrap_or(c"?".as_ptr()) };
    let n = c(name);
    unsafe { (h.emit_local)(plugin, n.as_ptr(), data.as_ptr() as *const c_char, data.len()) }
}

/// the running game's Scenario (read from it: "game._Mypair._Myval2...", class "Scenario"), None before a game runs
/// or with a core older than 0.10.0
pub fn scenario() -> Option<usize> {
    let h = host().filter(|_| core_version() >= (0, 10, 0))?;
    let s = unsafe { (h.scenario)() } as usize;
    (s != 0).then_some(s)
}

/// an engine object's value along `path`, as JSON (`class` "": the object's real class, from its vtable)
pub fn read(addr: usize, class: &str, path: &str, depth: u32) -> Result<String, String> {
    let h = host().filter(|_| core_version() >= (0, 7, 0)).ok_or("the core is older than 0.7.0: no read")?;
    let (cl, p) = (c(class), c(path));
    let mut out: *const c_char = std::ptr::null();
    let mut len = 0usize;
    let rc = unsafe { (h.read)(addr as u64, cl.as_ptr(), p.as_ptr(), depth, &mut out, &mut len) };
    let text = if out.is_null() { String::new() } else {
        String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(out as *const u8, len) }).into_owned()
    };
    if rc == 0 { Ok(text) } else { Err(text) }
}

// ---- hooking engine functions ----------------------------------------------------------------------------------

/// an MSVC-decorated name made readable (flags 0: the full prototype; 0x1000: just the name)
pub fn undecorate(name: &str, flags: u32) -> String {
    let Ok(c) = std::ffi::CString::new(name) else { return name.into() };
    let mut buf = [0u8; 2048];
    let n = unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::UnDecorateSymbolName(c.as_ptr() as *const u8, buf.as_mut_ptr(),
                                                                             buf.len() as u32, flags)
    };
    if n == 0 { name.into() } else { String::from_utf8_lossy(&buf[..n as usize]).into_owned() }
}

/// can a wrapper passing `max` integer arguments through (registers and stack slots, never float registers) stand
/// in for this function? Err says why not.
/// `proto` is the full prototype: "public: void __cdecl TransportLine::update(class MapTick,unsigned int) __ptr64"
pub fn hookable(proto: &str, max: usize) -> Result<(), String> {
    let p = proto.replace(" __ptr64", "");
    let at = p.find("__cdecl ").ok_or("no prototype")?;
    let open = p[at..].find('(').map(|j| at + j).ok_or("no prototype")?;
    let close = p.rfind(')').ok_or("no prototype")?;
    let ret = &p[..at];
    if ret.contains("float") || ret.contains("double") {
        return Err("returns a float".into());
    }
    let mut args = Vec::new();
    let (mut depth, mut cur) = (0i32, String::new());
    for ch in p[open + 1..close].chars() {
        match ch {
            '<' | '(' => {
                depth += 1;
                cur.push(ch)
            }
            '>' | ')' => {
                depth -= 1;
                cur.push(ch)
            }
            ',' if depth == 0 => args.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    args.push(cur);
    let args: Vec<&str> = args.iter().map(|a| a.trim()).filter(|a| !a.is_empty() && *a != "void").collect();
    if args.iter().any(|a| a.contains("...")) {
        return Err("variadic".into());
    }
    if args.iter().any(|a| (a.contains("float") || a.contains("double")) && !a.contains('*') && !a.contains('&')) {
        return Err("takes a float".into());
    }
    let member = ["public:", "private:", "protected:"].iter().any(|s| p.starts_with(s)) && !p.contains(" static ");
    // (a class returned by value comes back through a hidden pointer argument)
    let hidden = (ret.contains("class ") || ret.contains("struct ")) && !ret.contains('*') && !ret.contains('&');
    let n = args.len() + member as usize + hidden as usize;
    if n > max { Err(format!("{n} arguments (at most {max})")) } else { Ok(()) }
}

thread_local! {
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// hand a result to the host through the thread's output buffer (valid until this thread's next call)
pub unsafe fn answer(r: Result<Vec<u8>, String>, out: *mut *const c_char, out_len: *mut usize) -> i32 {
    let (rc, bytes) = match r {
        Ok(b) => (0, b),
        Err(e) => (1, e.into_bytes()),
    };
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        *o = bytes;
        *out = o.as_ptr() as *const c_char;
        *out_len = o.len();
    });
    rc
}

/// the input of a plugin call as text
pub unsafe fn input(p: *const c_char, len: usize) -> String {
    if p.is_null() { String::new() } else { String::from_utf8_lossy(std::slice::from_raw_parts(p as *const u8, len)).into_owned() }
}

/// a plugin function from a closure `|name: &str, input: &str| -> Result<String, String>` (panics become errors)
#[macro_export]
macro_rules! export {
    ($fname:ident, $body:expr) => {
        unsafe extern "C" fn $fname(_ud: *mut ::std::ffi::c_void, name: *const ::std::ffi::c_char,
                                    input: *const ::std::ffi::c_char, len: usize,
                                    out: *mut *const ::std::ffi::c_char, out_len: *mut usize) -> i32 {
            let name = ::std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned();
            let text = $crate::input(input, len);
            let f: fn(&str, &str) -> Result<String, String> = $body;
            let r = ::std::panic::catch_unwind(|| f(&name, &text))
                .unwrap_or_else(|_| Err(format!("panic in {}", stringify!($fname))));
            $crate::answer(r.map(String::into_bytes), out, out_len)
        }
    };
}
