//! fnative core: injected into factorio.exe by the launcher.
//!
//! On load it reads the engine's own symbols from factorio.pdb (shipped next to the exe), finds the Lua C API
//! compiled into the game, and detours `luaopen_base` so every Lua state Factorio opens (settings, data, control of
//! every mod) gets a global `native` table. Lua mods call into native code through it; they must treat it as
//! optional (`if native then ... end`) so the game and saves work without the loader.
//!
//! Rules for native functions called from Lua:
//! - never raise a Lua error (Factorio's Lua is C++ and unwinds with exceptions; unwinding through Rust frames is
//!   undefined): return `nil, message` instead;
//! - never keep a `lua_State` pointer past the call.

mod engine;
mod json;
mod log;
mod lua;
mod plugins;
mod symbols;

use std::ffi::c_void;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{BOOL, HMODULE, TRUE};
use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows_sys::Win32::System::Threading::{GetCurrentProcessId, OpenEventW, SetEvent, EVENT_MODIFY_STATE};

pub use lua::LuaApi;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

static API: OnceLock<LuaApi> = OnceLock::new();
static SYMBOLS: OnceLock<symbols::Symbols> = OnceLock::new();

pub fn api() -> &'static LuaApi {
    API.get().expect("fnative not initialised")
}

pub fn symbols() -> &'static symbols::Symbols {
    SYMBOLS.get().expect("fnative not initialised")
}

#[no_mangle]
pub extern "system" fn DllMain(_module: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        // (DllMain runs under the loader lock: the work happens on a thread of its own; the launcher keeps the game's
        // main thread suspended until that thread signals it is done)
        std::thread::spawn(|| {
            let ok = match init() {
                Ok(()) => true,
                Err(e) => {
                    log::line(&format!("init failed: {e}"));
                    false
                }
            };
            signal_ready(ok);
        });
    }
    TRUE
}

fn init() -> Result<(), String> {
    let started = std::time::Instant::now();
    log::line(&format!("fnative {VERSION} loading"));
    let pdb = std::env::current_exe().map_err(|e| e.to_string())?.with_extension("pdb");
    let syms = symbols::Symbols::new(fnative_engine::functions(&pdb)?);
    log::line(&format!("{} engine symbols read in {:.2?}", syms.len(), started.elapsed()));
    // (same build as the pdb? what everything needs there? a new build gets its report: see engine.rs)
    let core_needs = fnative_engine::Needs {
        functions: lua::NEEDED.iter().map(|s| s.to_string()).chain(Some("luaopen_base".to_string())).collect(),
        classes: Default::default(),
    };
    engine::check(&pdb, syms.map(), core_needs)?;
    let api = LuaApi::resolve(&syms)?;
    let _ = SYMBOLS.set(syms);
    let _ = API.set(api);
    lua::install_hooks()?;
    plugins::load_all();
    log::line(&format!("ready in {:.2?}", started.elapsed()));
    Ok(())
}

fn signal_ready(ok: bool) {
    let name: Vec<u16> = format!("Local\\fnative-ready-{}", unsafe { GetCurrentProcessId() })
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let ev = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
        if !ev.is_null() {
            if !ok {
                log::line("the game starts without native support");
            }
            SetEvent(ev);
        }
    }
}
