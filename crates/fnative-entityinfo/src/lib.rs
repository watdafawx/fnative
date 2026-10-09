//! fnative plugin "entityinfo": rows of a mod's own in the game's info panel for the entity under the cursor (the
//! panel under the minimap), in the game's own look. A mod sets rows for one entity (its prototype name and unit
//! number) and keeps them current; they show whenever that entity is hovered, until cleared.
//!
//!   entityinfo.set     {"name": "crew-character", "unit": 123, "rows": [["Doing", "building"], ["Kills", "12"]]}
//!   entityinfo.clear   {"unit": 123}   (no unit: every row of every entity)
//!   entityinfo.status  {"hooked": true, "entities": 1, "shown": 40}
//!
//! How: every entity's info is a Description the engine fills through its virtual addToDescription (rebuilt each
//! frame while hovered). The hooks let that run, then, for an entity with rows, add them with
//! Description::add(name, value, flags), the call the engine's own rows use. Unit numbers are only read for entities
//! whose prototype name has rows (only entities with an owner have one).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use fnative_plugin as fp;
use retour::GenericDetour;

/// the classes whose addToDescription is hooked: not every override calls its base (Character's doesn't), so the
/// root alone misses some; rows go in at the outermost of them, after it ran, once
const CLASSES: [&str; 6] = ["Entity", "EntityWithHealth", "EntityWithOwner", "Character", "Car", "SpiderVehicle"];
const DESCRIPTION_ADD: &str =
    "?add@Description@@QEAAXV?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@0E@Z";
const OPERATOR_NEW: &str = "??2@YAPEAX_K@Z";

/// MSVC x64 std::string: a 16-byte inline buffer or a heap pointer, then size, then capacity
#[repr(C)]
struct StdString {
    buf: [u8; 16],
    size: usize,
    cap: usize,
}

type AddToDescription = unsafe extern "C" fn(usize, usize);
// (strings by value: the caller builds them, the callee destroys them, freeing a heap buffer with the game's delete)
type DescriptionAdd = unsafe extern "C" fn(usize, *mut StdString, *mut StdString, u8);
type OperatorNew = unsafe extern "C" fn(usize) -> *mut u8;

struct Engine {
    add: DescriptionAdd,
    new: OperatorNew,
    prototype: usize,
    name: usize,
    unit: usize,
}

static HOOKS: [OnceLock<GenericDetour<AddToDescription>>; 6] =
    [OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new()];
thread_local!(static DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) });
static ENGINE: OnceLock<Engine> = OnceLock::new();
/// unit number -> (prototype name, rows)
static ROWS: Mutex<Option<HashMap<u64, (String, Vec<(String, String)>)>>> = Mutex::new(None);
static SHOWN: AtomicU64 = AtomicU64::new(0);

/// a std::string the engine may own and free: inline when short, else a buffer from the game's own operator new
unsafe fn make_string(e: &Engine, s: &str) -> StdString {
    let b = &s.as_bytes()[..s.len().min(4000)]; // (under 4 KB: MSVC frees bigger ones with an aligned scheme)
    let mut out = StdString { buf: [0; 16], size: b.len(), cap: 15 };
    if b.len() <= 15 {
        out.buf[..b.len()].copy_from_slice(b);
    } else {
        let p = (e.new)(b.len() + 1);
        std::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len());
        *p.add(b.len()) = 0;
        out.buf[..8].copy_from_slice(&(p as usize).to_le_bytes());
        out.cap = b.len();
    }
    out
}

unsafe fn read_string(p: usize) -> &'static [u8] {
    let size = *((p + 16) as *const usize);
    let cap = *((p + 24) as *const usize);
    if size > 4096 || cap < size {
        return &[];
    }
    let data = if cap >= 16 { *(p as *const usize) as *const u8 } else { p as *const u8 };
    std::slice::from_raw_parts(data, size)
}

unsafe fn described(i: usize, this: usize, desc: usize) {
    DEPTH.with(|d| d.set(d.get() + 1));
    HOOKS[i].get().expect("hook").call(this, desc);
    let outer = DEPTH.with(|d| {
        d.set(d.get() - 1);
        d.get() == 0
    });
    if outer {
        add_rows(this, desc);
    }
}

unsafe extern "C" fn h0(t: usize, d: usize) { described(0, t, d) }
unsafe extern "C" fn h1(t: usize, d: usize) { described(1, t, d) }
unsafe extern "C" fn h2(t: usize, d: usize) { described(2, t, d) }
unsafe extern "C" fn h3(t: usize, d: usize) { described(3, t, d) }
unsafe extern "C" fn h4(t: usize, d: usize) { described(4, t, d) }
unsafe extern "C" fn h5(t: usize, d: usize) { described(5, t, d) }
const DETOURS: [AddToDescription; 6] = [h0, h1, h2, h3, h4, h5];

unsafe fn add_rows(this: usize, desc: usize) {
    let (Some(e), true) = (ENGINE.get(), this != 0 && desc != 0) else { return };
    let rows = ROWS.lock().unwrap();
    let Some(rows) = rows.as_ref().filter(|r| !r.is_empty()) else { return };
    let proto = *((this + e.prototype) as *const usize);
    if proto == 0 {
        return;
    }
    let name = read_string(proto + e.name);
    if !rows.values().any(|(n, _)| n.as_bytes() == name) {
        return;
    }
    let unit = *((this + e.unit) as *const u64);
    let Some((n, list)) = rows.get(&unit) else { return };
    if n.as_bytes() != name {
        return;
    }
    for (k, v) in list {
        let (mut a, mut b) = (make_string(e, k), make_string(e, v));
        (e.add)(desc, &mut a, &mut b, 0);
    }
    if SHOWN.fetch_add(1, Ordering::Relaxed) == 0 {
        fp::log(&format!("first rows shown: {} #{unit}", n));
    }
}

fn parse(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text).map_err(|e| format!("bad JSON: {e}"))
}

fp::export!(f_set, |_, text| {
    let v = parse(text)?;
    let name = v["name"].as_str().ok_or("no name")?.to_string();
    let unit = v["unit"].as_u64().ok_or("no unit")?;
    let mut list = Vec::new();
    for r in v["rows"].as_array().ok_or("no rows")? {
        let k = r[0].as_str().unwrap_or("").to_string();
        let val = r[1].as_str().map(str::to_string).unwrap_or_else(|| r[1].to_string());
        list.push((k, val));
    }
    ROWS.lock().unwrap().get_or_insert_with(HashMap::new).insert(unit, (name, list));
    Ok("ok".into())
});

fp::export!(f_clear, |_, text| {
    let v = parse(if text.is_empty() { "{}" } else { text })?;
    let mut rows = ROWS.lock().unwrap();
    match (v["unit"].as_u64(), rows.as_mut()) {
        (Some(u), Some(r)) => {
            r.remove(&u);
        }
        (None, Some(r)) => r.clear(),
        _ => {}
    }
    Ok("ok".into())
});

fp::export!(f_status, |_, _| {
    let n = ROWS.lock().unwrap().as_ref().map(|r| r.len()).unwrap_or(0);
    let hooked: Vec<&str> = CLASSES.iter().zip(HOOKS.iter()).filter(|(_, h)| h.get().is_some()).map(|(c, _)| *c).collect();
    Ok(format!("{{\"hooked\":{:?},\"entities\":{},\"shown\":{}}}", hooked, n, SHOWN.load(Ordering::Relaxed)))
});

unsafe fn install() -> Result<(), String> {
    let sym = |n: &str| fp::engine_symbol(n).ok_or(format!("not in this build: {n}"));
    let field = |c: &str, f: &str| fp::field_offset(c, f).map(|o| o as usize).ok_or(format!("no {c}::{f}"));
    let engine = Engine {
        add: std::mem::transmute::<usize, DescriptionAdd>(sym(DESCRIPTION_ADD)?),
        new: std::mem::transmute::<usize, OperatorNew>(sym(OPERATOR_NEW)?),
        prototype: field("Entity", "prototype")?,
        name: field("PrototypeBase", "name")?,
        unit: field("EntityWithOwner", "unitNumber")?,
    };
    let _ = ENGINE.set(engine);
    let mut n = 0;
    for (i, class) in CLASSES.iter().enumerate() {
        let name = format!("?addToDescription@{class}@@UEBAXAEAVDescription@@@Z");
        let Some(addr) = fp::engine_symbol(&name) else {
            fp::log(&format!("not in this build: {class}::addToDescription"));
            continue;
        };
        let target = std::mem::transmute::<usize, AddToDescription>(addr);
        match GenericDetour::<AddToDescription>::new(target, DETOURS[i]).and_then(|d| d.enable().map(|_| d)) {
            Ok(d) => {
                let _ = HOOKS[i].set(d);
                n += 1;
            }
            Err(err) => fp::log(&format!("hook {class}::addToDescription: {err}")),
        }
    }
    if n == 0 {
        return Err("no addToDescription hooked".into());
    }
    Ok(())
}

#[no_mangle]
pub unsafe extern "C" fn fnative_plugin_init(host: *const fp::Host) -> i32 {
    if !fp::init(host, "entityinfo") {
        return 1;
    }
    fp::register("entityinfo", "set", f_set, 0);
    fp::register("entityinfo", "clear", f_clear, 0);
    fp::register("entityinfo", "status", f_status, 0);
    match install() {
        Ok(()) => fp::log("hooked addToDescription: mods' rows go into the entity info panel"),
        Err(e) => fp::log(&format!("off: {e}")),
    }
    0
}
