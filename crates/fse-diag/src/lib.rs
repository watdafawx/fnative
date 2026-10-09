//! fse plugin "diag": the engine often catches its own exceptions and says only "not used" or "failed"; this
//! logs the message of every RuntimeError / DeserialiserException it constructs, with the engine call stack as
//! RVAs (`fse-pdb` or the symbol map names them), to fse.log. Exceptions are rare, so it costs nothing.
//! Off unless FSE_DIAG=1 (dist/fse.env).
//!
//!   diag.recent   the last 200 messages as JSON [{"msg", "stack": [rva, ...]}]

use std::sync::{Mutex, OnceLock};

use fse_plugin as fp;
use retour::GenericDetour;
use windows_sys::Win32::System::Diagnostics::Debug::RtlCaptureStackBackTrace;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

// (every hook passes all four argument registers through untouched, whatever the function's own signature says)
type Ctor = unsafe extern "C" fn(usize, usize, usize, usize) -> usize;

static DESER_STR: OnceLock<GenericDetour<Ctor>> = OnceLock::new();
static RT_STR: OnceLock<GenericDetour<Ctor>> = OnceLock::new();
static RT_CSTR: OnceLock<GenericDetour<Ctor>> = OnceLock::new();
static RECENT: Mutex<Vec<(String, Vec<usize>)>> = Mutex::new(Vec::new());
// the startup data cache's checks (why "Data stage cache not used"): every package it checksums, and its abort
static CRCS: OnceLock<GenericDetour<Ctor>> = OnceLock::new();
static ABORT: OnceLock<GenericDetour<Ctor>> = OnceLock::new();
static PACKAGE_NAME: OnceLock<Option<u64>> = OnceLock::new();

/// MSVC x64 std::string: 16-byte inline buffer or heap pointer, then size, then capacity
unsafe fn std_string(p: usize) -> String {
    if p == 0 {
        return String::new();
    }
    let size = *((p + 16) as *const usize);
    let cap = *((p + 24) as *const usize);
    if size > 1 << 20 || cap < size {
        return "<unreadable>".into();
    }
    let data = if cap >= 16 { *(p as *const usize) as *const u8 } else { p as *const u8 };
    String::from_utf8_lossy(std::slice::from_raw_parts(data, size)).into_owned()
}

unsafe fn c_string(p: usize) -> String {
    if p == 0 { String::new() } else { std::ffi::CStr::from_ptr(p as *const i8).to_string_lossy().into_owned() }
}

fn record(kind: &str, msg: String) {
    let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
    let mut frames = [std::ptr::null_mut(); 12];
    let n = unsafe { RtlCaptureStackBackTrace(2, frames.len() as u32, frames.as_mut_ptr(), std::ptr::null_mut()) } as usize;
    // (engine frames only, as offsets into factorio.exe)
    let stack: Vec<usize> = frames[..n].iter().map(|f| *f as usize).filter(|a| *a > base && *a < base + 0x4000000)
        .map(|a| a - base).collect();
    let mut r = RECENT.lock().unwrap();
    if r.last().map(|(m, _)| m == &msg).unwrap_or(false) {
        return; // (DeserialiserException builds a RuntimeError with the same text)
    }
    fp::log(&format!("{kind}: {msg} | stack {}", stack.iter().map(|a| format!("{a:x}")).collect::<Vec<_>>().join(" ")));
    r.push((msg, stack));
    if r.len() > 200 {
        r.remove(0);
    }
}

unsafe extern "C" fn deser_str(this: usize, s: usize, c: usize, d: usize) -> usize {
    record("DeserialiserException", std_string(s));
    DESER_STR.get().unwrap().call(this, s, c, d)
}

unsafe extern "C" fn rt_str(this: usize, s: usize, c: usize, d: usize) -> usize {
    record("RuntimeError", std_string(s));
    RT_STR.get().unwrap().call(this, s, c, d)
}

unsafe extern "C" fn rt_cstr(this: usize, s: usize, c: usize, d: usize) -> usize {
    record("RuntimeError", c_string(s));
    RT_CSTR.get().unwrap().call(this, s, c, d)
}

/// ModDataCache::calculateCrcs(result, const PackagePath&): static, the result map by a hidden pointer; a
/// PackagePath starts with its Package*
unsafe extern "C" fn crcs(ret: usize, path: usize, c: usize, d: usize) -> usize {
    let package = if path != 0 { *(path as *const usize) } else { 0 };
    let name = PACKAGE_NAME.get().copied().flatten().filter(|_| package != 0)
        .map(|o| std_string(package + o as usize)).unwrap_or_default();
    let r = CRCS.get().unwrap().call(ret, path, c, d);
    // the result: std::map<Path, u32> {head, size}; a node is left, parent, right, colour/isnil, then the Path (a
    // 32-byte std::wstring) at 32 and its CRC at 64
    let mut files = Vec::new();
    let head = *(ret as *const usize);
    let size = *((ret + 8) as *const usize);
    let mut stack = Vec::new();
    let mut node = *((head + 8) as *const usize); // (the root: head's parent)
    while (node != head || !stack.is_empty()) && files.len() <= size {
        while node != head {
            stack.push(node);
            node = *(node as *const usize);
        }
        let Some(n) = stack.pop() else { break };
        files.push(format!("{}={:08x}", std_wstring(n + 32), *((n + 64) as *const u32)));
        node = *((n + 16) as *const usize);
    }
    fp::log(&format!("data cache: checking the files of {name}: {}", files.join(" ")));
    r
}

/// MSVC x64 std::wstring: an 8-wchar inline buffer or heap pointer, then size, then capacity
unsafe fn std_wstring(p: usize) -> String {
    let size = *((p + 16) as *const usize);
    let cap = *((p + 24) as *const usize);
    if size > 1 << 16 || cap < size {
        return "<unreadable>".into();
    }
    let data = if cap >= 8 { *(p as *const usize) as *const u16 } else { p as *const u16 };
    String::from_utf16_lossy(std::slice::from_raw_parts(data, size))
}

unsafe extern "C" fn abort_cache(a: usize, b: usize, c: usize, d: usize) -> usize {
    fp::log("data cache: ABORT (the package checked last differs from the cache)");
    ABORT.get().unwrap().call(a, b, c, d)
}

// FSE_DIAG_GC=1: every lua_gc(L, what, data) the engine makes, counted by `what` with its time and how many
// different Lua states it touched, logged every 200 000 calls (what drives "luaGarbageIncremental")
type GcFn = unsafe extern "C" fn(usize, i32, i32) -> i32;
static GC: OnceLock<GenericDetour<GcFn>> = OnceLock::new();
struct GcStats {
    calls: [u64; 16],
    nanos: [u64; 16],
    states: std::collections::HashSet<usize>,
    datas: std::collections::HashMap<(i32, i32), u64>,
    total: u64,
}
static GC_STATS: Mutex<Option<GcStats>> = Mutex::new(None);

unsafe extern "C" fn gc_hook(l: usize, what: i32, data: i32) -> i32 {
    let t = std::time::Instant::now();
    let r = GC.get().unwrap().call(l, what, data);
    let ns = t.elapsed().as_nanos() as u64;
    let mut g = GC_STATS.lock().unwrap();
    let s = g.get_or_insert_with(|| GcStats { calls: [0; 16], nanos: [0; 16], states: Default::default(),
                                              datas: Default::default(), total: 0 });
    let w = (what.clamp(0, 15)) as usize;
    s.calls[w] += 1;
    s.nanos[w] += ns;
    s.states.insert(l);
    *s.datas.entry((what, data)).or_default() += 1;
    s.total += 1;
    if s.total % 5_000 == 0 {
        let by: Vec<String> = (0..16).filter(|&i| s.calls[i] > 0)
            .map(|i| format!("what={i}: {} calls {:.1} ms", s.calls[i], s.nanos[i] as f64 / 1e6)).collect();
        let mut d: Vec<_> = s.datas.iter().collect();
        d.sort_by(|a, b| b.1.cmp(a.1));
        let top: Vec<String> = d.iter().take(5).map(|((w, x), n)| format!("({w},{x})x{n}")).collect();
        fp::log(&format!("gc: {} calls, {} Lua states | {} | top (what,data): {}", s.total, s.states.len(),
                         by.join(", "), top.join(" ")));
        s.calls = [0; 16];
        s.nanos = [0; 16];
        s.datas.clear();
    }
    r
}

fp::export!(f_recent, |_, _| {
    let r = RECENT.lock().unwrap();
    let items: Vec<String> = r.iter().map(|(m, s)| format!("{{\"msg\":{:?},\"stack\":{:?}}}", m, s)).collect();
    Ok(format!("[{}]", items.join(",")))
});

unsafe fn hook(slot: &OnceLock<GenericDetour<Ctor>>, name: &str, f: Ctor) {
    let Some(addr) = fp::engine_symbol(name) else {
        fp::log(&format!("not in this build: {name}"));
        return;
    };
    match GenericDetour::<Ctor>::new(std::mem::transmute::<usize, Ctor>(addr), f).and_then(|d| d.enable().map(|_| d)) {
        Ok(d) => {
            let _ = slot.set(d);
        }
        Err(e) => fp::log(&format!("hook {name}: {e}")),
    }
}

#[no_mangle]
pub unsafe extern "C" fn fse_plugin_init(host: *const fp::Host) -> i32 {
    if !fp::init(host, "diag") {
        return 1;
    }
    if std::env::var("FSE_DIAG").map(|v| v != "1").unwrap_or(true) {
        return 0; // (off unless asked for)
    }
    hook(&DESER_STR, "??0DeserialiserException@@QEAA@AEBV?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@@Z", deser_str);
    hook(&RT_STR, "??0RuntimeError@@QEAA@AEBV?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@@Z", rt_str);
    hook(&RT_CSTR, "??0RuntimeError@@QEAA@PEBD@Z", rt_cstr);
    if std::env::var("FSE_DIAG_CACHE").map(|v| v == "1").unwrap_or(false) {
        let _ = PACKAGE_NAME.set(fp::field_offset("Package", "name_"));
        hook(&CRCS, "?calculateCrcs@ModDataCache@@CA?AV?$map@UPath@Filesystem@@IU?$less@UPath@Filesystem@@@std@@V?$allocator@U?$pair@$$CBUPath@Filesystem@@I@std@@@4@@std@@AEBVPackagePath@@@Z", crcs);
        if let Some(addr) = fp::engine_symbol("?abort@ModDataCache@@SAXAEAVCacheData@1@@Z") {
            if let Ok(d) = GenericDetour::<Ctor>::new(std::mem::transmute::<usize, Ctor>(addr), abort_cache)
                .and_then(|d| d.enable().map(|_| d)) {
                let _ = ABORT.set(d);
            }
        }
    }
    if std::env::var("FSE_DIAG_GC").map(|v| v == "1").unwrap_or(false) {
        if let Some(addr) = fp::engine_symbol("lua_gc") {
            if let Ok(d) = GenericDetour::<GcFn>::new(std::mem::transmute::<usize, GcFn>(addr), gc_hook)
                .and_then(|d| d.enable().map(|_| d)) {
                let _ = GC.set(d);
            }
        }
    }
    fp::register("diag", "recent", f_recent, fp::THREADSAFE);
    fp::log("engine exception messages are logged (FSE_DIAG=1)");
    0
}
