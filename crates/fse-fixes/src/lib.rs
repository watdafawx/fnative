//! fse plugin "fixes": small in-memory fixes to engine bugs. Each one names the engine function it sits in and
//! the exact instruction bytes it expects there; when a game update changes them the fix is skipped (and logged), never
//! guessed. Nothing on disk changes. All on by default; FSE_FIXES=0 turns them all off, FSE_FIX_<NAME>=0 one.
//!
//!   fixes.list   JSON [{"name", "applied", "note"}]
//!
//! data-cache-tilde: with `cache-prototype-data=true` (config.ini) the game keeps the data stage in data-cache.dat and
//!   skips the mods' data Lua at the next start. But the cache does not save a dependency's "~" (does not affect load
//!   order) flag, and its check compares it, so any enabled mod with a "~" dependency makes it "Data stage cache not
//!   used" at every start. The fix leaves that one flag out of the check (the sorted mod list it would change is checked
//!   on its own).

use std::sync::Mutex;

use fse_plugin as fp;
use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE};

struct Fix {
    name: &'static str,
    function: &'static str,
    /// how far into the function to look
    span: usize,
    /// the bytes expected (None: any byte)
    pattern: &'static [Option<u8>],
    /// where in the pattern to write, and what
    at: usize,
    write: &'static [u8],
}

const FIXES: &[Fix] = &[Fix {
    name: "data-cache-tilde",
    function: "?loadInternal@ModDataCache@@CA_NAEAVCacheData@1@@Z",
    span: 0x1000,
    // movzx eax, byte [rbx+6]; cmp [r14+rbx+6], al; jne <mismatch>; add rbx, 0x30  (affectsSorting, the last field
    // the dependency check compares); the jne becomes a 6-byte nop
    pattern: &[Some(0x0f), Some(0xb6), Some(0x43), Some(0x06), Some(0x41), Some(0x38), Some(0x44), Some(0x1e), Some(0x06),
               Some(0x0f), Some(0x85), None, None, None, None, Some(0x48), Some(0x83), Some(0xc3), Some(0x30)],
    at: 9,
    write: &[0x66, 0x0f, 0x1f, 0x44, 0x00, 0x00],
}];

static DONE: Mutex<Vec<(&'static str, bool, String)>> = Mutex::new(Vec::new());

// ---- gc-idle-skip ------------------------------------------------------------------------------------------------
// Every tick the engine gives each mod's Lua state 48 KB of GC "debt" and forces a step (LuaContext::update), so with
// hundreds of mods every state collects all the time, even one that allocated nothing: a finished cycle restarts and
// walks its whole heap again (2-3 ms a tick on a 400-mod save). Skipped here: a forced step of a state that is
// between cycles (paused) and whose allocated total hasn't changed since its last cycle ended - it has no new garbage.
// A state that allocates (or frees) gets its steps again; a cycle under way always finishes. The debt the engine
// added is taken back, so nothing piles up for later. The engine's layouts come from the PDB (needs file).
type StepFn = unsafe extern "C" fn(usize);
static STEP: std::sync::OnceLock<retour::GenericDetour<StepFn>> = std::sync::OnceLock::new();
struct GcOffsets { l_g: usize, total: usize, debt: usize, state: usize, kind: usize, blocked: usize }
static GC_OFF: std::sync::OnceLock<GcOffsets> = std::sync::OnceLock::new();
// global_State* -> (total when its last cycle ended, totalbytes and GCdebt then)
static IDLE: Mutex<Option<std::collections::HashMap<usize, (i64, u64, i64)>>> = Mutex::new(None);
static SKIPPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static STEPPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const GCS_PAUSE: u8 = 5;

unsafe extern "C" fn forcestep(l: usize) {
    let o = GC_OFF.get().unwrap();
    let g = *((l + o.l_g) as *const usize);
    let total = |g: usize| (*((g + o.total) as *const u64) as i64).wrapping_add(*((g + o.debt) as *const i64));
    let normal = *((g + o.kind) as *const u8) == 0 && *((g + o.blocked) as *const u8) == 0;
    if normal && *((g + o.state) as *const u8) == GCS_PAUSE {
        let mut idle = IDLE.lock().unwrap();
        if let Some(&(t, tb, d)) = idle.get_or_insert_with(Default::default).get(&g) {
            if t == total(g) {
                // (nothing allocated since the last cycle: undo the engine's debt, skip the step)
                *((g + o.total) as *mut u64) = tb;
                *((g + o.debt) as *mut i64) = d;
                SKIPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        }
    }
    STEP.get().unwrap().call(l);
    let n = STEPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if n % 500_000 == 0 {
        fp::log(&format!("gc-idle-skip: {} steps skipped, {n} run", SKIPPED.load(std::sync::atomic::Ordering::Relaxed)));
    }
    if normal && *((g + o.state) as *const u8) == GCS_PAUSE {
        let tb = *((g + o.total) as *const u64);
        let d = *((g + o.debt) as *const i64);
        IDLE.lock().unwrap().get_or_insert_with(Default::default).insert(g, (total(g), tb, d));
    }
}

unsafe fn gc_idle_skip() -> Result<String, String> {
    let f = |c: &str, n: &str| fp::field_offset(c, n).map(|v| v as usize).ok_or(format!("{c}.{n} not in this build"));
    let off = GcOffsets { l_g: f("lua_State", "l_G")?, total: f("global_State", "totalbytes")?,
        debt: f("global_State", "GCdebt")?, state: f("global_State", "gcstate")?,
        kind: f("global_State", "gckind")?, blocked: f("global_State", "gcblocked")? };
    let addr = fp::engine_symbol("?luaC_forcestep@@YAXPEAUlua_State@@@Z").ok_or("luaC_forcestep not in this build")?;
    let _ = GC_OFF.set(off);
    let d = retour::GenericDetour::<StepFn>::new(std::mem::transmute::<usize, StepFn>(addr), forcestep)
        .and_then(|d| d.enable().map(|_| d)).map_err(|e| format!("hook: {e}"))?;
    let _ = STEP.set(d);
    Ok("luaC_forcestep hooked".into())
}

fp::export!(f_gc, |_, _| {
    use std::sync::atomic::Ordering::Relaxed;
    Ok(format!("{{\"skipped\":{},\"stepped\":{}}}", SKIPPED.load(Relaxed), STEPPED.load(Relaxed)))
});

unsafe fn apply(f: &Fix) -> Result<String, String> {
    let start = fp::engine_symbol(f.function).ok_or("its function is not in this build")?;
    let code = std::slice::from_raw_parts(start as *const u8, f.span);
    let hits: Vec<usize> = (0..=f.span - f.pattern.len())
        .filter(|&i| f.pattern.iter().enumerate().all(|(j, b)| b.map(|b| code[i + j] == b).unwrap_or(true)))
        .collect();
    let [i] = hits[..] else {
        return Err(format!("its code changed in this build ({} matches): skipped", hits.len()));
    };
    let addr = start + i + f.at;
    let mut old = 0;
    if VirtualProtect(addr as *const _, f.write.len(), PAGE_EXECUTE_READWRITE, &mut old) == 0 {
        return Err("could not make the code writable".into());
    }
    std::ptr::copy_nonoverlapping(f.write.as_ptr(), addr as *mut u8, f.write.len());
    VirtualProtect(addr as *const _, f.write.len(), old, &mut old);
    Ok(format!("at {}+{:#x}", f.function, i + f.at))
}

fp::export!(f_list, |_, _| {
    let d = DONE.lock().unwrap();
    let items: Vec<String> = d.iter()
        .map(|(n, ok, note)| format!("{{\"name\":{n:?},\"applied\":{ok},\"note\":{note:?}}}")).collect();
    Ok(format!("[{}]", items.join(",")))
});

#[no_mangle]
pub unsafe extern "C" fn fse_plugin_init(host: *const fp::Host) -> i32 {
    if !fp::init(host, "fixes") {
        return 1;
    }
    let off = |k: &str| std::env::var(k).map(|v| v == "0").unwrap_or(false);
    {
        let r = if off("FSE_FIXES") || off("FSE_FIX_GC_IDLE_SKIP") { Err("off (FSE_FIX_GC_IDLE_SKIP=0)".into()) }
                else { gc_idle_skip() };
        match &r {
            Ok(n) => fp::log(&format!("gc-idle-skip: applied, {n}")),
            Err(e) => fp::log(&format!("gc-idle-skip: not applied, {e}")),
        }
        DONE.lock().unwrap().push(("gc-idle-skip", r.is_ok(), r.unwrap_or_else(|e| e)));
    }
    for f in FIXES {
        let key = format!("FSE_FIX_{}", f.name.to_uppercase().replace('-', "_"));
        let r = if off("FSE_FIXES") || off(&key) { Err(format!("off ({key}=0)")) } else { apply(f) };
        match &r {
            Ok(note) => fp::log(&format!("{}: applied {note}", f.name)),
            Err(e) => fp::log(&format!("{}: not applied, {e}", f.name)),
        }
        let ok = r.is_ok();
        DONE.lock().unwrap().push((f.name, ok, r.unwrap_or_else(|e| e)));
    }
    fp::register("fixes", "list", f_list, fp::THREADSAFE);
    fp::register("fixes", "gc", f_gc, fp::THREADSAFE);
    0
}
