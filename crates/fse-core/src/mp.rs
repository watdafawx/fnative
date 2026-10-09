//! Multiplayer without desyncs. Factorio's multiplayer is lockstep: every peer simulates the same ticks from the same
//! input actions, so anything a peer knows locally (the mouse, a file, Python's answer, the time) must not change the
//! game unless it went through the input actions first. Two ways in:
//!
//! - Events from inside the simulation (an engine hook firing during the update): every peer's engine runs the same
//!   hooked calls in the same tick. They are collected while Scenario::updateStep runs and handed to Lua right after
//!   it, in a fixed order, still inside the tick: fse-std raises them as the "fse-event" event. (Delivering them in
//!   the next tick's on_tick would break on join: the joining peer loads a save taken between the two.)
//! - Local data: native.sync(name, data) sends it as a console command input action ("/fse-sync ..."), the game's
//!   own way for a player's input to reach every peer; fse-std raises "fse-sync" on all of them in the same tick.
//!
//! What a plugin keeps (hooks, blocked inputs) isn't in the save: mods set it again in on_load, so a joining peer has
//! it before its first tick.

use std::ffi::c_int;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};

use retour::GenericDetour;

use crate::lua::{push_str, push_value, LuaState};
use crate::{api, log, symbols};

const UPDATE_STEP: &str = "?updateStep@Scenario@@QEAAXXZ";
const LUA_CLOSE: &str = "lua_close";
const ACTION_CTOR: &str =
    "??0InputAction@@AEAA@W4InputActionType@@AEBV?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@@Z";
const ACTION_DTOR: &str = "??1InputAction@@QEAA@XZ";
const SEND: &str = "?tryToSendInputAction@Scenario@@QEBA_N$$QEAVInputAction@@@Z";

type UpdateStep = unsafe extern "C" fn(usize);
type Close = unsafe extern "C" fn(*mut LuaState);
type Ctor = unsafe extern "C" fn(*mut u8, u32, *const StdString) -> *mut u8;
type Dtor = unsafe extern "C" fn(*mut u8);
type Send = unsafe extern "C" fn(usize, *mut u8) -> bool;

static STEP: OnceLock<GenericDetour<UpdateStep>> = OnceLock::new();
static CLOSE: OnceLock<GenericDetour<Close>> = OnceLock::new();
static SCENARIO: AtomicUsize = AtomicUsize::new(0);
static UPDATING: AtomicBool = AtomicBool::new(false);
/// simulation events of the running tick: (plugin, name, data)
static SIM: Mutex<Vec<(String, String, String)>> = Mutex::new(Vec::new());
/// Lua states that take the simulation events at the end of each tick (their global __fse_tick_end)
static RECEIVERS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// is the simulation updating right now? (events emitted then belong to it)
pub fn updating() -> bool {
    UPDATING.load(Relaxed)
}

pub fn sim_event(plugin: &str, name: &str, data: &str) {
    SIM.lock().unwrap().push((plugin.into(), name.into(), data.into()));
}

unsafe extern "C" fn update_step(scenario: usize) {
    SCENARIO.store(scenario, Relaxed);
    SIM.lock().unwrap().clear();
    UPDATING.store(true, Relaxed);
    STEP.get().unwrap_unchecked().call(scenario);
    UPDATING.store(false, Relaxed);
    deliver();
}

/// the tick's simulation events, sorted (threads emit them in any order; every peer must see the same order), to
/// each receiving Lua state
unsafe fn deliver() {
    let mut events = std::mem::take(&mut *SIM.lock().unwrap());
    if events.is_empty() {
        return;
    }
    events.sort();
    let receivers = RECEIVERS.lock().unwrap().clone();
    let a = api();
    for l in receivers {
        let l = l as *mut LuaState;
        let top = (a.gettop)(l);
        (a.getglobal)(l, c"__fse_tick_end".as_ptr());
        if (a.type_of)(l, -1) != 6 {
            (a.settop)(l, top); // (LUA_TFUNCTION is 6: nothing to call)
            continue;
        }
        (a.createtable)(l, events.len() as c_int, 0);
        for (i, (p, n, d)) in events.iter().enumerate() {
            (a.createtable)(l, 0, 3);
            push_str(l, p);
            (a.setfield)(l, -2, c"plugin".as_ptr());
            push_str(l, n);
            (a.setfield)(l, -2, c"name".as_ptr());
            match serde_json::from_str::<serde_json::Value>(d) {
                Ok(v) => push_value(l, &v, 0),
                Err(_) => push_str(l, d),
            }
            (a.setfield)(l, -2, c"data".as_ptr());
            (a.rawseti)(l, -2, i as c_int + 1);
        }
        if (a.pcallk)(l, 1, 0, 0, 0, None) != 0 {
            log::line(&format!("fse-event delivery: {}", crate::lua::arg_str(l, -1).unwrap_or_default()));
        }
        (a.settop)(l, top);
    }
}

unsafe extern "C" fn close(l: *mut LuaState) {
    RECEIVERS.lock().unwrap().retain(|&x| x != l as usize);
    CLOSE.get().unwrap_unchecked().call(l)
}

pub fn install_hooks() -> Result<(), String> {
    let s = symbols();
    unsafe {
        let step = s.addr(UPDATE_STEP).ok_or("no Scenario::updateStep")?;
        let d = GenericDetour::<UpdateStep>::new(std::mem::transmute::<usize, UpdateStep>(step), update_step)
            .map_err(|e| e.to_string())?;
        d.enable().map_err(|e| e.to_string())?;
        let _ = STEP.set(d);
        let c = s.addr(LUA_CLOSE).ok_or("no lua_close")?;
        let d = GenericDetour::<Close>::new(std::mem::transmute::<usize, Close>(c), close).map_err(|e| e.to_string())?;
        d.enable().map_err(|e| e.to_string())?;
        let _ = CLOSE.set(d);
    }
    Ok(())
}

/// what the core needs for this (checked against each build)
pub const NEEDED: &[&str] = &[UPDATE_STEP, LUA_CLOSE, ACTION_CTOR, ACTION_DTOR, SEND];

// ---- Lua -----------------------------------------------------------------------------------------------------------

/// native.on_tick_end() : this Lua state's global __fse_tick_end(events) gets each tick's simulation events (fse-std
/// does this; one receiver is enough, it raises "fse-event" for every mod)
pub unsafe extern "C" fn n_on_tick_end(l: *mut LuaState) -> c_int {
    let mut r = RECEIVERS.lock().unwrap();
    if !r.contains(&(l as usize)) {
        r.push(l as usize);
    }
    0
}

/// MSVC std::string (only read by the engine here: it copies it)
#[repr(C)]
struct StdString {
    buf: [u8; 16],
    size: usize,
    cap: usize,
}

/// native.sync(name, data) -> true | nil, error : `data` (a string, JSON by convention) reaches every peer as the
/// "fse-sync" event {player_index, key = name, data}, in the same tick. Only a peer with a player can send (not a headless
/// server). Keep it small: it travels as a console command.
pub unsafe extern "C" fn n_sync(l: *mut LuaState) -> c_int {
    let (Some(name), Some(data)) = (crate::lua::arg_str(l, 1), crate::lua::arg_str(l, 2)) else {
        return crate::lua::fail(l, "native.sync(name, data): two strings");
    };
    if name.is_empty() || name.contains(char::is_whitespace) {
        return crate::lua::fail(l, "native.sync: the name can't have spaces");
    }
    match send_command(&format!("/fse-sync {name} {data}")) {
        Ok(()) => {
            (api().pushboolean)(l, 1);
            1
        }
        Err(e) => crate::lua::fail(l, &e),
    }
}

fn send_command(text: &str) -> Result<(), String> {
    let scenario = SCENARIO.load(Relaxed);
    if scenario == 0 {
        return Err("no game running".into());
    }
    if local_player().is_none() {
        return Err("no local player (a headless server can't send)".into());
    }
    let kind = crate::engine::with_types(|t| t.enum_value("InputActionType", "WriteToConsole")
        .ok_or("no InputActionType::WriteToConsole".to_string()))?;
    let s = symbols();
    let (ctor, dtor, send) = (s.addr(ACTION_CTOR).ok_or("no InputAction constructor")?,
                              s.addr(ACTION_DTOR).ok_or("no InputAction destructor")?,
                              s.addr(SEND).ok_or("no Scenario::tryToSendInputAction")?);
    let bytes = text.as_bytes();
    let mut heap = bytes.to_vec();
    heap.push(0);
    let mut st = StdString { buf: [0; 16], size: bytes.len(), cap: 15 };
    if bytes.len() <= 15 {
        st.buf[..bytes.len()].copy_from_slice(bytes);
    } else {
        st.buf[..8].copy_from_slice(&(heap.as_ptr() as usize).to_le_bytes());
        st.cap = bytes.len();
    }
    let size = crate::engine::with_types(|t| t.size_of("InputAction").ok_or("no InputAction".to_string()))?;
    // (8-aligned room for the action, with spare)
    let mut room = vec![0u64; (size as usize).div_ceil(8) + 4];
    unsafe {
        let action = room.as_mut_ptr() as *mut u8;
        std::mem::transmute::<usize, Ctor>(ctor)(action, kind as u32, &st);
        // (the constructor leaves the sender unset: this peer's own player, counted from 0 like the game does)
        let at = crate::engine::with_types(|t| t.field_offset("InputAction", "playerIndex")
            .ok_or("no InputAction::playerIndex".to_string()))?;
        *(action.add(at as usize) as *mut u16) = (local_player().unwrap_or(1) - 1) as u16;
        let ok = std::mem::transmute::<usize, Send>(send)(scenario, action);
        std::mem::transmute::<usize, Dtor>(dtor)(action);
        if !ok {
            return Err("the game didn't take the input action".into());
        }
    }
    drop(heap);
    Ok(())
}

/// the local player's index (as Lua counts, from 1), or None (headless, menu)
fn local_player() -> Option<u64> {
    let scenario = SCENARIO.load(Relaxed);
    if scenario == 0 {
        return None;
    }
    crate::engine::with_types(|t| t.read(scenario, "Scenario", "game._Mypair._Myval2.localPlayer.index", 0))
        .ok().and_then(|v| v.as_u64()).map(|i| i + 1)
}

/// native.local_player() -> the index of this peer's own player, or nil (a headless server; the menu). Local
/// knowledge: use it to decide what this peer sends, never to change the game directly.
pub unsafe extern "C" fn n_local_player(l: *mut LuaState) -> c_int {
    match local_player() {
        Some(i) => (api().pushnumber)(l, i as f64),
        None => (api().pushnil)(l),
    }
    1
}
