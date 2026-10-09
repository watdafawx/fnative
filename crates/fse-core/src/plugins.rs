//! Native plugins (see include/fse.h): loaded from the plugins folder before the game starts, called from Lua
//! through native.call (game thread) or native.start / native.poll (worker threads).

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH};

use crate::{log, symbols};

pub const ABI: u32 = 1;
pub const THREADSAFE: u32 = 1;

pub type PluginFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, usize, *mut *const c_char,
                                         *mut usize) -> i32;

#[repr(C)]
pub struct Host {
    abi: u32,
    version: *const c_char,
    log: unsafe extern "C" fn(*const c_char, *const c_char),
    engine_symbol: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    register_fn: unsafe extern "C" fn(*const c_char, *const c_char, PluginFn, *mut c_void, u32) -> i32,
    // (appended in ABI 1 rev 2: older plugins read only the fields above)
    build: *const c_char,
    field_offset: unsafe extern "C" fn(*const c_char, *const c_char) -> i64,
    class_size: unsafe extern "C" fn(*const c_char) -> i64,
    // (core 0.4.0)
    call: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, usize, *mut *const c_char, *mut usize) -> i32,
    // (core 0.7.0)
    emit: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, usize),
    read: unsafe extern "C" fn(u64, *const c_char, *const c_char, u32, *mut *const c_char, *mut usize) -> i32,
    // (core 0.8.0)
    emit_local: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, usize),
}

// (the version string it points to is a static CString: read-only, lives for the whole process)
unsafe impl Send for Host {}
unsafe impl Sync for Host {}

#[derive(Clone, Copy)]
struct Entry {
    f: PluginFn,
    ud: usize, // (a plugin's own pointer: only passed back to it)
    flags: u32,
}

static REGISTRY: Mutex<Option<HashMap<(String, String), Entry>>> = Mutex::new(None);

unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
}

unsafe extern "C" fn host_log(plugin: *const c_char, msg: *const c_char) {
    log::line(&format!("[{}] {}", cstr(plugin), cstr(msg)));
}

unsafe extern "C" fn host_engine_symbol(name: *const c_char) -> *mut c_void {
    symbols().addr(&cstr(name)).map(|a| a as *mut c_void).unwrap_or(std::ptr::null_mut())
}

unsafe extern "C" fn host_register(plugin: *const c_char, name: *const c_char, f: PluginFn, ud: *mut c_void,
                                   flags: u32) -> i32 {
    let key = (cstr(plugin), cstr(name));
    let mut reg = REGISTRY.lock().unwrap();
    let map = reg.get_or_insert_with(HashMap::new);
    if map.contains_key(&key) {
        return 1;
    }
    log::line(&format!("registered {}.{}{}", key.0, key.1, if flags & THREADSAFE != 0 { " (threadsafe)" } else { "" }));
    map.insert(key, Entry { f, ud: ud as usize, flags });
    0
}

unsafe extern "C" fn host_field_offset(class: *const c_char, field: *const c_char) -> i64 {
    crate::engine::field_offset(&cstr(class), &cstr(field)).map(|o| o as i64).unwrap_or(-1)
}

unsafe extern "C" fn host_class_size(class: *const c_char) -> i64 {
    crate::engine::class_size(&cstr(class)).map(|o| o as i64).unwrap_or(-1)
}

thread_local! {
    static CALL_OUT: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// one plugin calling another's function; the answer stays valid until this thread's next host call
unsafe extern "C" fn host_call(plugin: *const c_char, name: *const c_char, input: *const c_char, len: usize,
                               out: *mut *const c_char, out_len: *mut usize) -> i32 {
    let bytes = if input.is_null() { &[][..] } else { std::slice::from_raw_parts(input as *const u8, len) };
    let (rc, data) = match call(&cstr(plugin), &cstr(name), bytes) {
        Ok(b) => (0, b),
        Err(e) => (1, e.into_bytes()),
    };
    CALL_OUT.with(|o| {
        let mut o = o.borrow_mut();
        *o = data;
        *out = o.as_ptr() as *const c_char;
        *out_len = o.len();
    });
    rc
}

unsafe extern "C" fn host_emit(plugin: *const c_char, name: *const c_char, data: *const c_char, len: usize) {
    let bytes = if data.is_null() { &[][..] } else { std::slice::from_raw_parts(data as *const u8, len) };
    crate::events::emit(&cstr(plugin), &cstr(name), &String::from_utf8_lossy(bytes), false);
}

/// an event that never counts as a simulation event (from the render thread, the mouse, ...)
unsafe extern "C" fn host_emit_local(plugin: *const c_char, name: *const c_char, data: *const c_char, len: usize) {
    let bytes = if data.is_null() { &[][..] } else { std::slice::from_raw_parts(data as *const u8, len) };
    crate::events::emit(&cstr(plugin), &cstr(name), &String::from_utf8_lossy(bytes), true);
}

/// an engine object's value along a path, as JSON (class NULL or "": the object's own class, from its vtable)
unsafe extern "C" fn host_read(addr: u64, class: *const c_char, path: *const c_char, depth: u32,
                               out: *mut *const c_char, out_len: *mut usize) -> i32 {
    let (class, path) = (cstr(class), cstr(path));
    let r = crate::engine::with_types(|t| if class.is_empty() {
        t.read_dynamic(addr as usize, &path, depth)
    } else {
        t.read(addr as usize, &class, &path, depth)
    });
    let (rc, data) = match r {
        Ok(v) => (0, v.to_string().into_bytes()),
        Err(e) => (1, e.into_bytes()),
    };
    CALL_OUT.with(|o| {
        let mut o = o.borrow_mut();
        *o = data;
        *out = o.as_ptr() as *const c_char;
        *out_len = o.len();
    });
    rc
}

static VERSION_C: OnceLock<CString> = OnceLock::new();
static BUILD_C: OnceLock<CString> = OnceLock::new();
static HOST: OnceLock<Host> = OnceLock::new();

/// the plugins folder: FSE_PLUGINS, else "plugins" beside the launcher (FSE_HOME)
fn folder() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("FSE_PLUGINS") {
        return Some(p.into());
    }
    std::env::var("FSE_HOME").ok().map(|h| std::path::Path::new(&h).join("plugins"))
}

pub fn load_all() {
    let version = VERSION_C.get_or_init(|| CString::new(crate::VERSION).unwrap());
    let host = HOST.get_or_init(|| Host {
        abi: ABI, version: version.as_ptr(), log: host_log, engine_symbol: host_engine_symbol,
        register_fn: host_register,
        build: BUILD_C.get_or_init(|| CString::new(crate::engine::engine().map(|e| e.build.to_string()).unwrap_or_default())
            .unwrap_or_default()).as_ptr(),
        field_offset: host_field_offset, class_size: host_class_size, call: host_call, emit: host_emit,
        read: host_read, emit_local: host_emit_local,
    });
    let Some(dir) = folder() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        log::line(&format!("no plugins folder at {}", dir.display()));
        return;
    };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path())
        .filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("dll")).unwrap_or(false)).collect();
    paths.sort();
    for p in paths {
        let w: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            // (altered search path: a plugin's own DLLs beside it are found first)
            let m = LoadLibraryExW(w.as_ptr(), std::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH);
            if m.is_null() {
                log::line(&format!("plugin {}: could not load ({})", p.display(), std::io::Error::last_os_error()));
                continue;
            }
            let Some(init) = GetProcAddress(m, c"fse_plugin_init".as_ptr() as *const u8) else {
                log::line(&format!("plugin {}: no fse_plugin_init export", p.display()));
                continue;
            };
            let init: unsafe extern "C" fn(*const Host) -> i32 = std::mem::transmute(init);
            let rc = init(host);
            log::line(&format!("plugin {}: init {}", p.display(), if rc == 0 { "ok".into() } else { format!("failed ({rc})") }));
        }
    }
}

fn lookup(plugin: &str, name: &str) -> Option<Entry> {
    let reg = REGISTRY.lock().unwrap();
    let map = reg.as_ref()?;
    map.get(&(plugin.to_string(), name.to_string()))
        .or_else(|| map.get(&(plugin.to_string(), "*".to_string())))
        .copied()
}

/// every plugin and its function names
pub fn list() -> Vec<(String, Vec<String>)> {
    let reg = REGISTRY.lock().unwrap();
    let mut by: HashMap<String, Vec<String>> = HashMap::new();
    for (p, n) in reg.as_ref().map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default() {
        by.entry(p).or_default().push(n);
    }
    let mut out: Vec<_> = by.into_iter().collect();
    out.sort();
    for (_, v) in out.iter_mut() {
        v.sort();
    }
    out
}

fn invoke(e: Entry, name: &str, input: &[u8]) -> Result<Vec<u8>, String> {
    let cname = CString::new(name).map_err(|_| "function name has a NUL byte".to_string())?;
    let mut out: *const c_char = std::ptr::null();
    let mut len = 0usize;
    let rc = unsafe { (e.f)(e.ud as *mut c_void, cname.as_ptr(), input.as_ptr() as *const c_char, input.len(), &mut out, &mut len) };
    let bytes = if out.is_null() { Vec::new() } else { unsafe { std::slice::from_raw_parts(out as *const u8, len) }.to_vec() };
    if rc == 0 { Ok(bytes) } else { Err(String::from_utf8_lossy(&bytes).into_owned()) }
}

pub fn call(plugin: &str, name: &str, input: &[u8]) -> Result<Vec<u8>, String> {
    let e = lookup(plugin, name).ok_or_else(|| format!("no native function {plugin}.{name}"))?;
    invoke(e, name, input)
}

// ---- jobs: a plugin call on a worker thread ------------------------------------------------------------------

pub enum Job {
    Pending,
    Done(Result<Vec<u8>, String>),
}

static JOBS: Mutex<Option<HashMap<u64, Job>>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

pub fn start(plugin: &str, name: &str, input: Vec<u8>) -> Result<u64, String> {
    let e = lookup(plugin, name).ok_or_else(|| format!("no native function {plugin}.{name}"))?;
    if e.flags & THREADSAFE == 0 {
        return Err(format!("{plugin}.{name} is not registered threadsafe: use native.call"));
    }
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    JOBS.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, Job::Pending);
    let name = name.to_string();
    std::thread::spawn(move || {
        let r = invoke(e, &name, &input);
        JOBS.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, Job::Done(r));
    });
    Ok(id)
}

/// a finished job is handed out once and forgotten
pub fn poll(id: u64) -> Option<Job> {
    let mut jobs = JOBS.lock().unwrap();
    let map = jobs.get_or_insert_with(HashMap::new);
    match map.get(&id)? {
        Job::Pending => Some(Job::Pending),
        Job::Done(_) => map.remove(&id),
    }
}
