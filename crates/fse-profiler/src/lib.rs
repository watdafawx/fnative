//! fse plugin "profiler": how long the engine spends in chosen functions, per tick.
//!
//! plugins/profiler.json: {"tick": "<pdb name of the once-per-tick function>", "functions": ["<pdb names>", ...]}
//! (pdb names: `fse-pdb functions <part>` prints them). Every function is detoured once, when the plugin loads
//! (the game is still suspended then): nothing is patched while the game runs. Only functions whose arguments all
//! travel in integer registers (at most 4 counting `this`, no float or double, not variadic) are hooked: the
//! wrapper passes rcx, rdx, r8, r9 through untouched. Others are listed as skipped, with why.
//! Timing (rdtsc) only happens between start and stop; stopped, a hook costs a load and a jump.
//!
//! Functions (all threadsafe): start, stop, status, profile(consumer)
//!   profile: since this consumer's last call: ticks, and per function calls, ms and ms per tick (inclusive:
//!   a function's time includes the hooked functions it calls)

use std::arch::x86_64::_rdtsc;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};

use fse_plugin as fp;
use retour::GenericDetour;
use serde::{Deserialize, Serialize};

type Raw = unsafe extern "C" fn(usize, usize, usize, usize) -> usize;

struct Slot {
    name: String,
    readable: String,
    calls: AtomicU64,
    cycles: AtomicU64,
    detour: OnceLock<GenericDetour<Raw>>,
}

static SLOTS: OnceLock<Vec<Slot>> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static TICK: OnceLock<Option<usize>> = OnceLock::new(); // (index of the tick function's slot)
static CYCLES_PER_MS: OnceLock<f64> = OnceLock::new();
static SKIPPED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
type Baseline = (std::time::Instant, Vec<(u64, u64)>);
static BASELINES: Mutex<Option<HashMap<String, Baseline>>> = Mutex::new(None);

#[inline(always)]
unsafe fn hit(i: usize, a: usize, b: usize, c: usize, d: usize) -> usize {
    let slot = &SLOTS.get().unwrap_unchecked()[i];
    let orig = slot.detour.get().unwrap_unchecked();
    if !ACTIVE.load(Relaxed) {
        return orig.call(a, b, c, d);
    }
    let t0 = _rdtsc();
    let r = orig.call(a, b, c, d);
    slot.cycles.fetch_add(_rdtsc().wrapping_sub(t0), Relaxed);
    slot.calls.fetch_add(1, Relaxed);
    r
}

/// one wrapper per slot (a detour needs a plain function, not a closure): slot_fn::<N> times slot N
unsafe extern "C" fn slot_fn<const N: usize>(a: usize, b: usize, c: usize, d: usize) -> usize {
    hit(N, a, b, c, d)
}

macro_rules! slot_table {
    ($($n:literal),*) => { const SLOT_FNS: &[Raw] = &[$(slot_fn::<$n>),*]; };
}
slot_table!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
            29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47);

#[derive(Deserialize)]
struct Config {
    tick: String,
    functions: Vec<String>,
    /// time from the moment the plugin loads (startup work), not only after profiler.start
    #[serde(default)]
    start: bool,
}

fn install(cfg: &Config) {
    let mut names: Vec<String> = vec![cfg.tick.clone()];
    names.extend(cfg.functions.iter().filter(|f| **f != cfg.tick).cloned());
    let mut chosen = Vec::new();
    let mut skipped = Vec::new();
    for name in names {
        if chosen.len() >= SLOT_FNS.len() {
            skipped.push((name, format!("more than {} functions", SLOT_FNS.len())));
            continue;
        }
        let Some(addr) = fp::engine_symbol(&name) else {
            skipped.push((name, "not in this build".into()));
            continue;
        };
        let proto = fp::undecorate(&name, 0);
        if let Err(why) = fp::hookable(&proto, 4) {
            skipped.push((name, format!("{why}: {proto}")));
            continue;
        }
        let readable = fp::undecorate(&name, 0x1000);
        chosen.push((Slot { name, readable, calls: AtomicU64::new(0), cycles: AtomicU64::new(0), detour: OnceLock::new() }, addr));
    }
    let _ = TICK.set(chosen.iter().position(|(s, _)| s.name == cfg.tick));
    let (slots, addrs): (Vec<Slot>, Vec<usize>) = chosen.into_iter().unzip();
    let _ = SLOTS.set(slots);
    for (i, addr) in addrs.into_iter().enumerate() {
        let slot = &SLOTS.get().unwrap()[i];
        let made = unsafe {
            GenericDetour::<Raw>::new(std::mem::transmute::<usize, Raw>(addr), SLOT_FNS[i]).and_then(|d| d.enable().map(|_| d))
        };
        match made {
            Ok(d) => {
                let _ = slot.detour.set(d);
            }
            Err(e) => skipped.push((slot.name.clone(), format!("detour failed: {e}"))),
        }
    }
    for (n, why) in &skipped {
        fp::log(&format!("skipped {n}: {why}"));
    }
    *SKIPPED.lock().unwrap() = skipped;
}

fn calibrate() -> f64 {
    let t = std::time::Instant::now();
    let c0 = unsafe { _rdtsc() };
    std::thread::sleep(std::time::Duration::from_millis(40));
    let c1 = unsafe { _rdtsc() };
    (c1 - c0) as f64 / (t.elapsed().as_secs_f64() * 1000.0)
}

fn counters() -> Vec<(u64, u64)> {
    SLOTS.get().map(|s| s.iter().map(|x| (x.calls.load(Relaxed), x.cycles.load(Relaxed))).collect()).unwrap_or_default()
}

#[derive(Serialize)]
struct Row {
    name: String,
    calls: u64,
    calls_per_tick: f64,
    ms: f64,
    ms_per_tick: f64,
    us_per_call: f64,
}

fn profile(consumer: &str) -> Result<String, String> {
    let now = counters();
    let slots = SLOTS.get().ok_or("not loaded")?;
    let (since, old) = {
        let mut b = BASELINES.lock().unwrap();
        let map = b.get_or_insert_with(HashMap::new);
        map.insert(consumer.to_string(), (std::time::Instant::now(), now.clone()))
            .unwrap_or((std::time::Instant::now(), vec![(0, 0); now.len()]))
    };
    let cpm = *CYCLES_PER_MS.get().unwrap_or(&1.0);
    let tick = TICK.get().copied().flatten();
    let ticks = tick.map(|i| now[i].0 - old[i].0).unwrap_or(0);
    let mut rows: Vec<Row> = slots.iter().enumerate().map(|(i, s)| {
        let calls = now[i].0 - old[i].0;
        let ms = (now[i].1 - old[i].1) as f64 / cpm;
        Row {
            name: s.readable.clone(), calls,
            calls_per_tick: if ticks > 0 { calls as f64 / ticks as f64 } else { 0.0 },
            ms, ms_per_tick: if ticks > 0 { ms / ticks as f64 } else { 0.0 },
            us_per_call: if calls > 0 { ms * 1000.0 / calls as f64 } else { 0.0 },
        }
    }).collect();
    rows.sort_by(|a, b| b.ms.partial_cmp(&a.ms).unwrap_or(std::cmp::Ordering::Equal));
    Ok(serde_json::json!({
        "active": ACTIVE.load(Relaxed), "seconds": since.elapsed().as_secs_f64(), "ticks": ticks,
        "tick_function": tick.map(|i| slots[i].readable.clone()), "functions": rows,
    }).to_string())
}

fp::export!(f_start, |_, _| {
    ACTIVE.store(true, Relaxed);
    Ok("started".into())
});
fp::export!(f_stop, |_, _| {
    ACTIVE.store(false, Relaxed);
    Ok("stopped".into())
});
fp::export!(f_profile, |_, input| profile(if input.is_empty() { "default" } else { input }));
fp::export!(f_status, |_, _| {
    let hooked: Vec<String> = SLOTS.get()
        .map(|s| s.iter().filter(|x| x.detour.get().is_some()).map(|x| x.readable.clone()).collect()).unwrap_or_default();
    let skipped: Vec<serde_json::Value> = SKIPPED.lock().unwrap().iter()
        .map(|(n, w)| serde_json::json!({"name": n, "why": w})).collect();
    Ok(serde_json::json!({"active": ACTIVE.load(Relaxed), "hooked": hooked, "skipped": skipped,
                          "cycles_per_ms": CYCLES_PER_MS.get(), "build": fp::build()}).to_string())
});

#[no_mangle]
pub unsafe extern "C" fn fse_plugin_init(host: *const fp::Host) -> i32 {
    if !fp::init(host, "profiler") {
        return 1;
    }
    let home = std::env::var("FSE_HOME").map(std::path::PathBuf::from).unwrap_or_default();
    let dir = std::env::var("FSE_PLUGINS").map(std::path::PathBuf::from).unwrap_or_else(|_| home.join("plugins"));
    let path = dir.join("profiler.json");
    let cfg: Config = match std::fs::read_to_string(&path).map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
        Ok(c) => c,
        Err(e) => {
            fp::log(&format!("{}: {e}; nothing hooked", path.display()));
            return 0;
        }
    };
    let _ = CYCLES_PER_MS.set(calibrate());
    install(&cfg);
    if cfg.start {
        ACTIVE.store(true, Relaxed);
    }
    for (f, n) in [(f_start as fp::PluginFn, "start"), (f_stop, "stop"), (f_profile, "profile"), (f_status, "status")] {
        fp::register("profiler", n, f, fp::THREADSAFE);
    }
    let n = SLOTS.get().map(|s| s.iter().filter(|x| x.detour.get().is_some()).count()).unwrap_or(0);
    fp::log(&format!("{n} engine functions hooked (timing starts with profiler.start)"));
    0
}
