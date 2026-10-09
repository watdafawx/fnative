//! The Lua C API compiled into factorio.exe (Lua 5.2, modified by Wube), resolved by symbol name, and the detour
//! that gives every Lua state a global `native` table.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::OnceLock;

use retour::GenericDetour;

use crate::symbols::Symbols;
use crate::{api, log, plugins, symbols};

pub type LuaState = c_void;
pub type CFunction = unsafe extern "C" fn(*mut LuaState) -> c_int;

macro_rules! lua_api {
    ($($field:ident : $sym:literal => fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        /// function pointers into the game's Lua
        pub struct LuaApi {
            $(pub $field: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }
        /// the engine functions the core resolves (its needs)
        pub const NEEDED: &[&str] = &[$($sym),*];
        impl LuaApi {
            pub fn resolve(s: &Symbols) -> Result<Self, String> {
                Ok(LuaApi {
                    $($field: unsafe {
                        std::mem::transmute::<usize, unsafe extern "C" fn($($arg),*) $(-> $ret)?>(
                            s.addr($sym).ok_or(concat!("missing engine symbol ", $sym))?)
                    },)*
                })
            }
        }
    };
}

lua_api! {
    gettop: "lua_gettop" => fn(*mut LuaState) -> c_int;
    settop: "lua_settop" => fn(*mut LuaState, c_int);
    type_of: "lua_type" => fn(*mut LuaState, c_int) -> c_int;
    tolstring: "lua_tolstring" => fn(*mut LuaState, c_int, *mut usize) -> *const c_char;
    tonumberx: "lua_tonumberx" => fn(*mut LuaState, c_int, *mut c_int) -> f64;
    pushnil: "lua_pushnil" => fn(*mut LuaState);
    pushboolean: "lua_pushboolean" => fn(*mut LuaState, c_int);
    pushnumber: "lua_pushnumber" => fn(*mut LuaState, f64);
    pushlstring: "lua_pushlstring" => fn(*mut LuaState, *const c_char, usize) -> *const c_char;
    pushcclosure: "lua_pushcclosure" => fn(*mut LuaState, CFunction, c_int);
    createtable: "lua_createtable" => fn(*mut LuaState, c_int, c_int);
    setfield: "lua_setfield" => fn(*mut LuaState, c_int, *const c_char);
    rawseti: "lua_rawseti" => fn(*mut LuaState, c_int, c_int);
    setglobal: "lua_setglobal" => fn(*mut LuaState, *const c_char);
    next: "lua_next" => fn(*mut LuaState, c_int) -> c_int;
    toboolean: "lua_toboolean" => fn(*mut LuaState, c_int) -> c_int;
    absindex: "lua_absindex" => fn(*mut LuaState, c_int) -> c_int;
    checkstack: "lua_checkstack" => fn(*mut LuaState, c_int) -> c_int;
}

pub const LUA_TBOOLEAN: c_int = 1;
pub const LUA_TNUMBER: c_int = 3;
pub const LUA_TSTRING: c_int = 4;
pub const LUA_TTABLE: c_int = 5;

// ---- helpers for native functions ----------------------------------------------------------------------------

/// argument `idx` as a string (numbers convert, as Lua does), or None
pub unsafe fn arg_str(l: *mut LuaState, idx: c_int) -> Option<String> {
    let t = (api().type_of)(l, idx);
    if t != LUA_TSTRING && t != LUA_TNUMBER {
        return None;
    }
    let mut len = 0usize;
    let p = (api().tolstring)(l, idx, &mut len);
    if p.is_null() {
        return None;
    }
    Some(String::from_utf8_lossy(std::slice::from_raw_parts(p as *const u8, len)).into_owned())
}

/// argument `idx` as raw bytes (a Lua string may hold any bytes), or None
pub unsafe fn arg_bytes(l: *mut LuaState, idx: c_int) -> Option<Vec<u8>> {
    let t = (api().type_of)(l, idx);
    if t != LUA_TSTRING && t != LUA_TNUMBER {
        return None;
    }
    let mut len = 0usize;
    let p = (api().tolstring)(l, idx, &mut len);
    (!p.is_null()).then(|| std::slice::from_raw_parts(p as *const u8, len).to_vec())
}

pub unsafe fn push_bytes(l: *mut LuaState, b: &[u8]) {
    (api().pushlstring)(l, b.as_ptr() as *const c_char, b.len());
}

pub unsafe fn arg_num(l: *mut LuaState, idx: c_int) -> Option<f64> {
    let mut ok: c_int = 0;
    let v = (api().tonumberx)(l, idx, &mut ok);
    (ok != 0).then_some(v)
}

pub unsafe fn push_str(l: *mut LuaState, s: &str) {
    (api().pushlstring)(l, s.as_ptr() as *const c_char, s.len());
}

/// `nil, message`: how native functions report failure (they never raise Lua errors)
pub unsafe fn fail(l: *mut LuaState, msg: &str) -> c_int {
    (api().pushnil)(l);
    push_str(l, msg);
    2
}

// ---- the native table --------------------------------------------------------------------------------------------

/// native.version() -> "0.1.0"
unsafe extern "C" fn n_version(l: *mut LuaState) -> c_int {
    push_str(l, crate::VERSION);
    1
}

/// native.log(text): a line in fse.log
unsafe extern "C" fn n_log(l: *mut LuaState) -> c_int {
    match arg_str(l, 1) {
        Some(s) => {
            log::line(&format!("lua: {s}"));
            0
        }
        None => fail(l, "native.log(text): text must be a string"),
    }
}

/// native.symbols(part, limit?) -> { names } : engine function names containing `part` (exploring the engine)
unsafe extern "C" fn n_symbols(l: *mut LuaState) -> c_int {
    let Some(part) = arg_str(l, 1) else { return fail(l, "native.symbols(part, limit?): part must be a string") };
    let limit = arg_num(l, 2).unwrap_or(50.0).clamp(1.0, 10_000.0) as usize;
    let names = symbols().find(&part, limit);
    (api().createtable)(l, names.len() as c_int, 0);
    for (i, n) in names.iter().enumerate() {
        push_str(l, n);
        (api().rawseti)(l, -2, i as c_int + 1);
    }
    1
}

/// the plugin and function names (arguments 1 and 2) and the input (argument 3, optional, "" by default)
unsafe fn call_args(l: *mut LuaState, what: &str) -> Result<(String, String, Vec<u8>), c_int> {
    let (Some(p), Some(f)) = (arg_str(l, 1), arg_str(l, 2)) else {
        return Err(fail(l, &format!("native.{what}(plugin, function, input?): plugin and function must be strings")));
    };
    let input = if (api().type_of)(l, 3) <= 0 { Vec::new() } else {
        match arg_bytes(l, 3) {
            Some(b) => b,
            None => return Err(fail(l, &format!("native.{what}: input must be a string (JSON by convention)"))),
        }
    };
    Ok((p, f, input))
}

/// native.call(plugin, function, input?) -> output | nil, error : on the game thread
unsafe extern "C" fn n_call(l: *mut LuaState) -> c_int {
    let (p, f, input) = match call_args(l, "call") { Ok(a) => a, Err(n) => return n };
    match plugins::call(&p, &f, &input) {
        Ok(out) => {
            push_bytes(l, &out);
            1
        }
        Err(e) => fail(l, &e),
    }
}

/// native.start(plugin, function, input?) -> job id | nil, error : on a worker thread (threadsafe functions)
unsafe extern "C" fn n_start(l: *mut LuaState) -> c_int {
    let (p, f, input) = match call_args(l, "start") { Ok(a) => a, Err(n) => return n };
    match plugins::start(&p, &f, input) {
        Ok(id) => {
            (api().pushnumber)(l, id as f64);
            1
        }
        Err(e) => fail(l, &e),
    }
}

/// native.poll(id) -> "pending" | "done", output | "error", message | nil, "unknown job" (a result is given once)
unsafe extern "C" fn n_poll(l: *mut LuaState) -> c_int {
    let Some(id) = arg_num(l, 1) else { return fail(l, "native.poll(id): id must be a number") };
    match plugins::poll(id as u64) {
        None => fail(l, "unknown job"),
        Some(plugins::Job::Pending) => {
            push_str(l, "pending");
            1
        }
        Some(plugins::Job::Done(Ok(out))) => {
            push_str(l, "done");
            push_bytes(l, &out);
            2
        }
        Some(plugins::Job::Done(Err(e))) => {
            push_str(l, "error");
            push_str(l, &e);
            2
        }
    }
}

/// native.to_json(value, skip?) -> JSON text | nil, error : any Lua value, in every stage (data.raw included), much
/// faster than helpers.table_to_json. `skip`: a table whose keys are field names left out at any depth (e.g. graphics).
/// Tables with keys 1..n become arrays, others objects (number keys as strings); functions and userdata become null.
unsafe extern "C" fn n_to_json(l: *mut LuaState) -> c_int {
    let mut skip = std::collections::HashSet::new();
    if (api().type_of)(l, 2) == LUA_TTABLE {
        (api().pushnil)(l);
        while (api().next)(l, 2) != 0 {
            if (api().type_of)(l, -2) == LUA_TSTRING {
                if let Some(k) = lua_string(l, -2) {
                    skip.insert(k);
                }
            }
            (api().settop)(l, -2);
        }
    }
    let mut out = String::with_capacity(1 << 16);
    match crate::json::write(l, 1, &mut out, &skip, 0) {
        Ok(()) => {
            push_str(l, &out);
            1
        }
        Err(e) => fail(l, &e),
    }
}

/// the string at `idx` without converting a number in place (that would break lua_next on a key)
pub unsafe fn lua_string(l: *mut LuaState, idx: c_int) -> Option<String> {
    if (api().type_of)(l, idx) != LUA_TSTRING {
        return None;
    }
    let mut len = 0usize;
    let p = (api().tolstring)(l, idx, &mut len);
    (!p.is_null()).then(|| String::from_utf8_lossy(std::slice::from_raw_parts(p as *const u8, len)).into_owned())
}

/// native.build() -> { build = "GUID-age", new = true on the first start of this build, report = path }
unsafe extern "C" fn n_build(l: *mut LuaState) -> c_int {
    let Some(e) = crate::engine::engine() else { return fail(l, "no engine description") };
    (api().createtable)(l, 0, 3);
    push_str(l, &e.build.to_string());
    (api().setfield)(l, -2, c"build".as_ptr());
    (api().pushboolean)(l, crate::engine::fresh() as c_int);
    (api().setfield)(l, -2, c"new".as_ptr());
    1
}

/// native.plugins() -> { plugin = { function names } }
unsafe extern "C" fn n_plugins(l: *mut LuaState) -> c_int {
    let all = plugins::list();
    (api().createtable)(l, 0, all.len() as c_int);
    for (p, fns) in all {
        (api().createtable)(l, fns.len() as c_int, 0);
        for (i, f) in fns.iter().enumerate() {
            push_str(l, f);
            (api().rawseti)(l, -2, i as c_int + 1);
        }
        let key = std::ffi::CString::new(p).unwrap_or_default();
        (api().setfield)(l, -2, key.as_ptr());
    }
    1
}

const FUNCTIONS: &[(&CStr, CFunction)] = &[
    (c"version", n_version),
    (c"log", n_log),
    (c"symbols", n_symbols),
    (c"call", n_call),
    (c"start", n_start),
    (c"poll", n_poll),
    (c"plugins", n_plugins),
    (c"build", n_build),
    (c"to_json", n_to_json),
];

unsafe fn register(l: *mut LuaState) {
    let a = api();
    (a.createtable)(l, 0, FUNCTIONS.len() as c_int);
    for (name, f) in FUNCTIONS {
        (a.pushcclosure)(l, *f, 0);
        (a.setfield)(l, -2, name.as_ptr());
    }
    (a.setglobal)(l, c"native".as_ptr());
}

// ---- the detour --------------------------------------------------------------------------------------------------

type OpenFn = unsafe extern "C" fn(*mut LuaState) -> c_int;
static OPEN_BASE: OnceLock<GenericDetour<OpenFn>> = OnceLock::new();

/// luaopen_base runs once for every Lua state the game makes: the original first, then the native table
unsafe extern "C" fn open_base_detour(l: *mut LuaState) -> c_int {
    let n = OPEN_BASE.get().expect("detour").call(l);
    register(l);
    n
}

pub fn install_hooks() -> Result<(), String> {
    let target = symbols().addr("luaopen_base").ok_or("missing engine symbol luaopen_base")?;
    unsafe {
        let original: OpenFn = std::mem::transmute(target);
        let d = GenericDetour::<OpenFn>::new(original, open_base_detour).map_err(|e| format!("detour: {e}"))?;
        d.enable().map_err(|e| format!("detour enable: {e}"))?;
        let _ = OPEN_BASE.set(d);
    }
    Ok(())
}
