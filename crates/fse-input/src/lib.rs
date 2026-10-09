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

#[no_mangle]
pub unsafe extern "C" fn fse_plugin_init(host: *const fp::Host) -> i32 {
    if !fp::init(host, "input") {
        return 1;
    }
    if fp::core_version() < (0, 7, 0) {
        fp::log("needs core 0.7.0: off");
        return 1;
    }
    for (f, n) in [(f_watch as fp::PluginFn, "watch"), (f_block, "block"), (f_status, "status")] {
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
