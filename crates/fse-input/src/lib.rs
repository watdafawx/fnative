//! fse plugin "input": the player input actions the game applies (all 336 kinds of InputActionType: building,
//! rotating, cursor splits, GUI clicks, train schedule edits, ...), as events for Lua, and chosen kinds blocked
//! before the game applies them. Every action passes GameActionHandler::actionPerformed(InputAction const&): that
//! one function is hooked.
//!
//!   input.watch   {"types": ["RotateEntity", ...]} or {"all": true}: send these as events "action"
//!                 {"type": "RotateEntity", "player": 1, "tick": 1234, "blocked": false}; {"types": []} stops
//!   input.block   {"types": ["RotateEntity"], "player": 1}: drop these actions (player left out: every player); a
//!                 blocked action is sent as an event too (blocked = true) when watched. {"types": []} unblocks all
//!   input.status  watched, blocked, counts
//!   input.controls  every control the game has (its own and each mod's custom inputs), with what is bound to it:
//!                 [{"name": "toggle-map", "modded": false, "keys": [{"scancode": "SDL_SCANCODE_M", "mods": []}, ...]}]
//!                 ("mouse": "Left"/... for mouse buttons). The player's own bindings, as Settings > Controls shows them
//!   input.trigger {"control": "toggle-map"}: presses the control's key into the game window (the std plugin's press),
//!                 so the game and every mod act as if the player pressed it. On this computer only: its input
//! The kinds: native.layout("InputActionType").values. Actions are applied on every peer, so blocking is safe in
//! multiplayer when every peer blocks the same kinds: call input.block from deterministic code, in on_init and on_load.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};

use fse_plugin as fp;
use retour::GenericDetour;
use serde_json::{json, Value};

const ACTION_PERFORMED: &str = "?actionPerformed@GameActionHandler@@UEAAXAEBVInputAction@@@Z";
type Performed = unsafe extern "C" fn(usize, usize);

static HOOK: OnceLock<GenericDetour<Performed>> = OnceLock::new();
static WATCH_ALL: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false); // (anything watched or blocked: else the hook only passes through)
static WATCHED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
/// blocked kinds -> the player they're blocked for (None: everyone)
static BLOCKED: Mutex<Vec<(String, Option<u64>)>> = Mutex::new(Vec::new());
static SEEN: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);

fn field(action: usize, path: &str) -> Value {
    fp::read(action, "InputAction", path, 0).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

fn refresh() {
    let any = WATCH_ALL.load(Relaxed) || WATCHED.lock().unwrap().as_ref().is_some_and(|w| !w.is_empty())
        || !BLOCKED.lock().unwrap().is_empty();
    ACTIVE.store(any, Relaxed);
}

unsafe extern "C" fn performed(this: usize, action: usize) {
    let orig = HOOK.get().unwrap_unchecked();
    if !ACTIVE.load(Relaxed) {
        return orig.call(this, action);
    }
    SEEN.fetch_add(1, Relaxed);
    let kind = field(action, "type");
    let kind_s = kind.as_str().unwrap_or("").to_string();
    let player = field(action, "playerIndex").as_u64();
    let blocked = BLOCKED.lock().unwrap().iter()
        .any(|(k, p)| *k == kind_s && (p.is_none() || *p == player));
    let watched = WATCH_ALL.load(Relaxed) || WATCHED.lock().unwrap().as_ref().is_some_and(|w| w.contains(&kind_s));
    if watched {
        // (playerIndex counts from 0; Lua's player.index from 1)
        let data = json!({"type": kind, "player": player.map(|p| p + 1), "tick": field(action, "updateTick"),
                          "blocked": blocked});
        fp::emit("action", &data.to_string());
    }
    if blocked {
        DROPPED.fetch_add(1, Relaxed);
        return;
    }
    orig.call(this, action)
}

fn parse(text: &str) -> Result<Value, String> {
    serde_json::from_str(if text.is_empty() { "{}" } else { text }).map_err(|e| format!("bad JSON: {e}"))
}

fn names(v: &Value) -> Vec<String> {
    v["types"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default()
}

fp::export!(f_watch, |_, text| {
    let v = parse(text)?;
    WATCH_ALL.store(v["all"].as_bool().unwrap_or(false), Relaxed);
    *WATCHED.lock().unwrap() = Some(names(&v).into_iter().collect());
    refresh();
    Ok("ok".into())
});
fp::export!(f_block, |_, text| {
    let v = parse(text)?;
    let kinds = names(&v);
    // (player numbers as Lua has them, from 1; stored from 0 like the game's)
    let player = v["player"].as_u64().map(|p| p.saturating_sub(1));
    let mut b = BLOCKED.lock().unwrap();
    if kinds.is_empty() {
        b.clear();
    } else {
        b.retain(|(k, _)| !kinds.contains(k));
        b.extend(kinds.into_iter().map(|k| (k, player)));
    }
    drop(b);
    refresh();
    Ok("ok".into())
});
fp::export!(f_status, |_, _| {
    let watched: Vec<String> = WATCHED.lock().unwrap().as_ref().map(|w| w.iter().cloned().collect()).unwrap_or_default();
    let blocked: Vec<Value> = BLOCKED.lock().unwrap().iter().map(|(k, p)| json!({"type": k, "player": p.map(|x| x + 1)})).collect();
    Ok(json!({"hooked": HOOK.get().is_some(), "all": WATCH_ALL.load(Relaxed), "watched": watched, "blocked": blocked,
              "seen": SEEN.load(Relaxed), "dropped": DROPPED.load(Relaxed)}).to_string())
});


// ---- the game's controls (key bindings) ---------------------------------------------------------------------------

const CONTROL_LIST: &str =
    "?getControlInputList@ControlInput@@SAAEAV?$vector@PEAVControlInput@@V?$allocator@PEAVControlInput@@@std@@@std@@XZ";
type ControlList = unsafe extern "C" fn() -> *const [usize; 3];

fn read_at(addr: usize, path: &str, depth: u32) -> Value {
    fp::read(addr, "ControlInput", path, depth).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

/// every ControlInput the game has (its own controls, then each mod's custom inputs)
fn control_ptrs() -> Result<Vec<usize>, String> {
    let addr = fp::engine_symbol(CONTROL_LIST).ok_or("ControlInput::getControlInputList is not in this build")?;
    let v = unsafe { &*std::mem::transmute::<usize, ControlList>(addr)() };
    let (b, e) = (v[0], v[1]);
    if b == 0 || e < b || (e - b) % 8 != 0 || (e - b) / 8 > 50_000 {
        return Err(format!("controls: the control list looks wrong ({b:#x}..{e:#x})"));
    }
    Ok((0..(e - b) / 8).map(|i| unsafe { *((b + i * 8) as *const usize) }).filter(|&p| p != 0).collect())
}

/// one binding (ControlInputValue) as {"scancode"} | {"mouse"} | ..., with "mods"; None when unbound
fn binding(v: &Value) -> Option<Value> {
    let kind = v["type"].as_str().unwrap_or("");
    let mods = v["modifiers"].as_u64().unwrap_or(0);
    let mut out = match kind {
        "Keyboard" => json!({"scancode": v["scancode"]}),
        // (mouse-button-N as the game names them: 1 left, 2 right, 3 middle, 4 and 5 the side buttons)
        k if k.contains("MouseButton") => json!({"mouse": format!("button-{}", v["mouseButton"].as_u64()?)}),
        k if k.contains("Wheel") => {
            let w = v["mouseWheel"].as_str()?.trim_start_matches("MouseWheel").to_ascii_lowercase();
            json!({"mouse": format!("wheel-{w}")})
        }
        _ => return None,
    };
    let names: Vec<&str> = MOD_BITS.iter().filter(|(b, _)| mods & b != 0).map(|&(_, n)| n).collect();
    out["mods"] = json!(names);
    out["modifiers"] = json!(mods);
    Some(out)
}

/// ControlInputValue::modifiers bits
const MOD_BITS: [(u64, &str); 3] = [(1, "ctrl"), (2, "shift"), (4, "alt")];

fn control(p: usize) -> Value {
    let name = read_at(p, "keyboardAndMouseInput1.key", 0);
    let k1 = read_at(p, "keyboardAndMouseInput1.value", 1);
    let k2 = read_at(p, "keyboardAndMouseInput2.value", 1);
    let custom = read_at(p, "customInputPrototype", 0);
    let keys: Vec<Value> = [binding(&k1), binding(&k2)].into_iter().flatten().collect();
    json!({"name": name, "keys": keys,
           "modded": !custom.is_null(), "category": read_at(p, "category", 1), "raw1": k1})
}

fn controls() -> Result<Vec<Value>, String> {
    Ok(control_ptrs()?.into_iter().map(control).collect())
}

fp::export!(f_controls, |_, text| {
    let all = controls()?;
    let raw = text.contains("raw");
    Ok(Value::Array(all.into_iter().map(|mut c| {
        if !raw {
            c.as_object_mut().map(|o| o.remove("raw1"));
        }
        c
    }).collect()).to_string())
});

// trigger {"control": "toggle-map", "hold_ms"?, "phase"?}: presses the control's first key binding into the game
// window (std.press), so the game and every mod see that control as if pressed here
fp::export!(f_trigger, |_, text| {
    let v = parse(text)?;
    let want = v["control"].as_str().ok_or("trigger: {\"control\": name}")?;
    let c = control_ptrs()?.into_iter().find(|&p| read_at(p, "keyboardAndMouseInput1.key", 0).as_str() == Some(want))
        .ok_or(format!("trigger: no control {want}"))?;
    let c = control(c);
    let mut key = c["keys"].get(0).cloned().ok_or(format!("trigger: {want} has no key bound"))?;
    for f in ["hold_ms", "phase"] {
        if !v[f].is_null() {
            key[f] = v[f].clone();
        }
    }
    let out = fp::call("std", "press", key.to_string().as_bytes())?;
    Ok(String::from_utf8_lossy(&out).into_owned())
});

#[no_mangle]
pub unsafe extern "C" fn fse_plugin_init(host: *const fp::Host) -> i32 {
    if !fp::init(host, "input") {
        return 1;
    }
    if fp::core_version() < (0, 7, 0) {
        fp::log("needs core 0.7.0: off");
        return 1;
    }
    for (f, n) in [(f_watch as fp::PluginFn, "watch"), (f_block, "block"), (f_status, "status"),
                   (f_controls, "controls"), (f_trigger, "trigger")] {
        fp::register("input", n, f, 0);
    }
    let Some(addr) = fp::engine_symbol(ACTION_PERFORMED) else {
        fp::log("GameActionHandler::actionPerformed is not in this build: off");
        return 0;
    };
    match GenericDetour::<Performed>::new(std::mem::transmute::<usize, Performed>(addr), performed)
        .and_then(|d| d.enable().map(|_| d)) {
        Ok(d) => {
            let _ = HOOK.set(d);
            fp::log("hooked GameActionHandler::actionPerformed");
        }
        Err(e) => fp::log(&format!("hook: {e}")),
    }
    0
}
