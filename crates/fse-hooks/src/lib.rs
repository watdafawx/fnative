//! fse plugin "hooks": engine functions, hooked by their factorio.pdb names, send every call (or every Nth) to Lua
//! as an event (native.events), with the arguments you choose read from the engine objects they point to.
//!
//! A hook: {"fn": "<pdb name>", "event": "<event name>", "every": 1, "result": false, "after": false,
//!          "args": [{"arg": 0, "path": "position", "class": "", "depth": 1, "as": "position"}, {"arg": 1, "raw": true}]}
//!   arg     which argument (0 is `this` for a member function; a class returned by value adds a hidden pointer)
//!   path    a field path from the object the argument points to ("" the object itself); class: the static class to
//!           read it as ("" its real class, from its vtable); depth: levels of nested objects; as: the key in the event
//!   raw     the argument's integer value itself
//!   after   read the arguments after the original ran (default: before); result: add its integer return value
//!   local   never part of the simulation: for functions outside the game update (rendering). Others raised during
//!           the update also arrive as that tick's "fse-event" on every multiplayer peer (fse-std)
//! Event data: {"args": {...}, "result": n, "calls": total}.
//!
//! Presets: {"preset": "console"} (every console message: {message = LocalisedString, from = player index from 0}),
//! "expansion" (where biters pick their next base: {position, from}), "save" (local: the path saved to), "app-state"
//! (local: the game's screen stack after a change: main menu, loading, in game...). Other fields override.
//!
//! Hooks come from plugins/hooks.json ({"hooks": [...]}, installed before the game starts) or from Lua at any time:
//!   hooks.add      a hook (JSON above) -> its event name   (patching while the game runs: fine for functions of the
//!                  game's update and Lua thread; avoid ones the render thread runs)
//!   hooks.enable / hooks.disable   {"event": name}   (a disabled hook costs a load and a jump)
//!   hooks.status   every hook: name, event, calls, enabled; and what was skipped and why
//! Only functions whose arguments are all integers or pointers (at most 8, no float or double, not variadic) can be
//! hooked: the wrapper passes four registers and four stack slots through untouched.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};

use fse_plugin as fp;
use retour::GenericDetour;
use serde::Deserialize;
use serde_json::{json, Map, Value};

type Raw = unsafe extern "C" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> usize;

#[derive(Deserialize, Clone)]
struct ArgSpec {
    arg: usize,
    #[serde(default)]
    path: String,
    #[serde(default)]
    class: String,
    #[serde(default = "one")]
    depth: u32,
    #[serde(default, rename = "as")]
    key: String,
    #[serde(default)]
    raw: bool,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize, Clone)]
struct Spec {
    #[serde(rename = "fn")]
    function: String,
    event: String,
    #[serde(default)]
    args: Vec<ArgSpec>,
    #[serde(default = "one_u64")]
    every: u64,
    #[serde(default)]
    result: bool,
    #[serde(default)]
    after: bool,
    /// never part of the simulation (a function the render thread runs): native.events only
    #[serde(default, rename = "local")]
    local: bool,
}

fn one_u64() -> u64 {
    1
}

struct Slot {
    spec: Spec,
    readable: String,
    calls: AtomicU64,
    enabled: AtomicBool,
    detour: OnceLock<GenericDetour<Raw>>,
}

const MAX: usize = 64;
static SLOTS: [OnceLock<Slot>; MAX] = [const { OnceLock::new() }; MAX];
static USED: AtomicUsize = AtomicUsize::new(0);
static SKIPPED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static INSTALL: Mutex<()> = Mutex::new(());

fn read_args(spec: &Spec, regs: &[usize; 8]) -> Map<String, Value> {
    let mut m = Map::new();
    for a in &spec.args {
        let key = if a.key.is_empty() { format!("arg{}{}", a.arg, if a.path.is_empty() { "".into() } else { format!(".{}", a.path) }) } else { a.key.clone() };
        let v = match regs.get(a.arg) {
            None => Value::Null,
            Some(&r) if a.raw => json!(r),
            Some(&r) => fp::read(r, &a.class, &a.path, a.depth)
                .ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null),
        };
        m.insert(key, v);
    }
    m
}

#[inline(always)]
unsafe fn hit(i: usize, r: [usize; 8]) -> usize {
    let slot = SLOTS[i].get().unwrap_unchecked();
    let orig = slot.detour.get().unwrap_unchecked();
    let n = slot.calls.fetch_add(1, Relaxed) + 1;
    if !slot.enabled.load(Relaxed) || n % slot.spec.every != 0 {
        return orig.call(r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]);
    }
    let before = (!slot.spec.after).then(|| read_args(&slot.spec, &r));
    let ret = orig.call(r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]);
    let args = before.unwrap_or_else(|| read_args(&slot.spec, &r));
    let mut data = json!({"args": args, "calls": n});
    if slot.spec.result {
        data["result"] = json!(ret);
    }
    if slot.spec.local {
        fp::emit_local(&slot.spec.event, &data.to_string());
    } else {
        fp::emit(&slot.spec.event, &data.to_string());
    }
    ret
}

unsafe extern "C" fn slot_fn<const N: usize>(a: usize, b: usize, c: usize, d: usize, e: usize, f: usize, g: usize,
                                             h: usize) -> usize {
    hit(N, [a, b, c, d, e, f, g, h])
}

macro_rules! slot_table {
    ($($n:literal),*) => { const SLOT_FNS: [Raw; MAX] = [$(slot_fn::<$n>),*]; };
}
slot_table!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
            29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55,
            56, 57, 58, 59, 60, 61, 62, 63);

/// hooks one function; Err says why not (also kept for status)
fn add(spec: Spec) -> Result<String, String> {
    let _g = INSTALL.lock().unwrap();
    let r = (|| {
        if spec.every == 0 {
            return Err("every must be at least 1".to_string());
        }
        if SLOTS.iter().filter_map(|s| s.get()).any(|s| s.spec.event == spec.event) {
            return Err(format!("event {} is taken", spec.event));
        }
        if SLOTS.iter().filter_map(|s| s.get()).any(|s| s.spec.function == spec.function) {
            return Err("already hooked (one hook per function)".into());
        }
        let addr = fp::engine_symbol(&spec.function).ok_or("not in this build")?;
        let proto = fp::undecorate(&spec.function, 0);
        fp::hookable(&proto, 8).map_err(|why| format!("{why}: {proto}"))?;
        let i = USED.load(Relaxed);
        if i >= MAX {
            return Err(format!("all {MAX} hooks are in use"));
        }
        let readable = fp::undecorate(&spec.function, 0x1000);
        let event = spec.event.clone();
        // (made before the slot is taken, enabled after: nothing calls the wrapper until then)
        let d = unsafe {
            GenericDetour::<Raw>::new(std::mem::transmute::<usize, Raw>(addr), SLOT_FNS[i])
                .map_err(|e| format!("detour: {e}"))?
        };
        let slot = Slot { spec, readable, calls: AtomicU64::new(0), enabled: AtomicBool::new(true), detour: OnceLock::from(d) };
        let _ = SLOTS[i].set(slot);
        USED.store(i + 1, Relaxed);
        let s = SLOTS[i].get().ok_or("slot")?;
        if let Err(e) = unsafe { s.detour.get().ok_or("detour")?.enable() } {
            s.enabled.store(false, Relaxed);
            return Err(format!("detour enable: {e}"));
        }
        fp::log(&format!("hooked {} -> event {event}", s.readable));
        Ok(event)
    })();
    r
}

fn find(event: &str) -> Option<&'static Slot> {
    SLOTS.iter().take(USED.load(Relaxed)).filter_map(|s| s.get()).find(|s| s.spec.event == event)
}

fn event_of(text: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("bad JSON: {e}"))?;
    v["event"].as_str().map(str::to_string).ok_or("no event".into())
}

/// ready-made hooks: {"preset": name} (plus any field to override, e.g. "event")
const PRESETS: &str = r#"{
  "console": {"fn": "?add@OutputConsole@@AEAAXAEBVLocalisedString@@PEBVPlayer@@AEBUPrintSettings@@$$QEAV?$vector@VSavedSpecialItemReference@@V?$allocator@VSavedSpecialItemReference@@@std@@@std@@@Z",
              "event": "console",
              "args": [{"arg": 1, "class": "LocalisedString", "depth": 4, "as": "message"},
                       {"arg": 2, "class": "Player", "path": "index", "as": "from"}]},
  "expansion": {"fn": "?findNewBasePosition@Commander@@AEBA?AV?$Optional@VMapPosition@@U?$OptionalEmptyValue@VMapPosition@@@@@@IAEBVMapPosition@@@Z",
                "event": "expansion", "after": true,
                "args": [{"arg": 1, "class": "Optional<MapPosition,OptionalEmptyValue<MapPosition> >", "depth": 2, "as": "position"},
                         {"arg": 3, "class": "MapPosition", "as": "from"}]},
  "save": {"fn": "?saveAs@Scenario@@QEAAXAEBUPath@Filesystem@@AEBV?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@0PEAVProgressObserver@@W4SaveType@@@Z",
           "event": "save", "local": true,
           "args": [{"arg": 1, "class": "Filesystem::Path", "depth": 2, "as": "path"},
                    {"arg": 2, "class": "std::basic_string<char,std::char_traits<char>,std::allocator<char> >", "as": "name"}]},
  "app-state": {"fn": "?changeStateInternal@AppManager@@AEAAXXZ",
                "event": "app-state", "local": true, "after": true,
                "args": [{"arg": 0, "class": "AppManager", "path": "stateStack", "depth": 1, "as": "states"}]}
}"#;

/// a hook's JSON with its preset filled in
fn expand(text: &str) -> Result<Spec, String> {
    let mut v: Value = serde_json::from_str(text).map_err(|e| format!("bad hook: {e}"))?;
    if let Some(name) = v.get("preset").and_then(|p| p.as_str()).map(str::to_string) {
        let presets: Value = serde_json::from_str(PRESETS).map_err(|e| e.to_string())?;
        let mut base = presets.get(&name).cloned().ok_or(format!("no preset {name} (there are {})",
            presets.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>().join(", ")).unwrap_or_default()))?;
        for (k, x) in v.as_object().into_iter().flatten().filter(|(k, _)| *k != "preset") {
            base[k] = x.clone();
        }
        v = base;
    }
    serde_json::from_value(v).map_err(|e| format!("bad hook: {e}"))
}

fp::export!(f_add, |_, text| {
    let spec = expand(text)?;
    let name = spec.function.clone();
    add(spec).inspect_err(|e| SKIPPED.lock().unwrap().push((name, e.clone())))
});
fp::export!(f_enable, |_, text| {
    find(&event_of(text)?).ok_or("no such hook")?.enabled.store(true, Relaxed);
    Ok("enabled".into())
});
fp::export!(f_disable, |_, text| {
    find(&event_of(text)?).ok_or("no such hook")?.enabled.store(false, Relaxed);
    Ok("disabled".into())
});
fp::export!(f_status, |_, _| {
    let hooks: Vec<Value> = SLOTS.iter().take(USED.load(Relaxed)).filter_map(|s| s.get()).map(|s| json!({
        "name": s.readable, "fn": s.spec.function, "event": s.spec.event, "calls": s.calls.load(Relaxed),
        "enabled": s.enabled.load(Relaxed),
    })).collect();
    let skipped: Vec<Value> = SKIPPED.lock().unwrap().iter().map(|(n, w)| json!({"fn": n, "why": w})).collect();
    Ok(json!({"hooks": hooks, "skipped": skipped}).to_string())
});

#[derive(Deserialize)]
struct Config {
    #[serde(default)]
    hooks: Vec<Value>,
}

#[no_mangle]
pub unsafe extern "C" fn fse_plugin_init(host: *const fp::Host) -> i32 {
    if !fp::init(host, "hooks") {
        return 1;
    }
    if fp::core_version() < (0, 7, 0) {
        fp::log("needs core 0.7.0 (events): off");
        return 1;
    }
    for (f, n) in [(f_add as fp::PluginFn, "add"), (f_enable, "enable"), (f_disable, "disable"), (f_status, "status")] {
        fp::register("hooks", n, f, fp::THREADSAFE);
    }
    let home = std::env::var("FSE_HOME").map(std::path::PathBuf::from).unwrap_or_default();
    let dir = std::env::var("FSE_PLUGINS").map(std::path::PathBuf::from).unwrap_or_else(|_| home.join("plugins"));
    if let Ok(text) = std::fs::read_to_string(dir.join("hooks.json")) {
        match serde_json::from_str::<Config>(&text) {
            Ok(cfg) => {
                for raw in cfg.hooks {
                    let spec = match expand(&raw.to_string()) {
                        Ok(s) => s,
                        Err(e) => {
                            fp::log(&format!("hooks.json: {e}"));
                            continue;
                        }
                    };
                    let name = spec.function.clone();
                    if let Err(e) = add(spec) {
                        fp::log(&format!("skipped {name}: {e}"));
                        SKIPPED.lock().unwrap().push((name, e));
                    }
                }
            }
            Err(e) => fp::log(&format!("hooks.json: {e}")),
        }
    }
    0
}
