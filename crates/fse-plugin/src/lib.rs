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
