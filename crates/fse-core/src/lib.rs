//! fse core: loaded into factorio.exe by the loader (version.dll in the game's folder) or injected by the launcher.
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
mod events;
mod inject;
mod json;
mod log;
mod lua;
mod mp;
mod mods;
mod plugins;
mod restart;
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
    API.get().expect("fse not initialised")
}

pub fn symbols() -> &'static symbols::Symbols {
    SYMBOLS.get().expect("fse not initialised")
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

/// FSE_NO_STEAM (the test scripts): Steam's init answers false, so the game runs like the standalone one and never
/// touches the player's settings in Steam's folder (the loader does the same for games it starts; see fse-loader)
fn no_steam() {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE};
    let name: Vec<u16> = "steam_api64.dll".encode_utf16().chain(Some(0)).collect();
    unsafe {
        let steam = GetModuleHandleW(name.as_ptr());
        if steam.is_null() {
            return;
        }
        for f in [c"SteamAPI_Init", c"SteamAPI_RestartAppIfNecessary"] {
            if let Some(addr) = GetProcAddress(steam, f.as_ptr() as *const u8) {
                let at = addr as usize as *mut u8;
                let mut old = 0u32;
                VirtualProtect(at as *const _, 3, PAGE_EXECUTE_READWRITE, &mut old);
                std::ptr::copy_nonoverlapping([0x31u8, 0xC0, 0xC3].as_ptr(), at, 3); // xor eax, eax ; ret
                VirtualProtect(at as *const _, 3, old, &mut old);
            }
        }
    }
}

fn init() -> Result<(), String> {
    let started = std::time::Instant::now();
    if std::env::var_os("FSE_NO_STEAM").is_some() {
        no_steam();
        log::line("FSE_NO_STEAM: the game runs without Steam");
    }
    log::line(&format!("fse {VERSION} loading"));
    let pdb = std::env::current_exe().map_err(|e| e.to_string())?.with_extension("pdb");
    let syms = symbols::Symbols::new(fse_engine::functions(&pdb)?);
    log::line(&format!("{} engine symbols read in {:.2?}", syms.len(), started.elapsed()));
    // (same build as the pdb? what everything needs there? a new build gets its report: see engine.rs)
    let core_needs = fse_engine::Needs {
        functions: lua::NEEDED.iter().chain(mp::NEEDED).map(|s| s.to_string()).chain(Some("luaopen_base".to_string()))
            .collect(),
        classes: Default::default(),
    };
    engine::check(&pdb, syms.map(), core_needs)?;
    let api = LuaApi::resolve(&syms)?;
    let _ = SYMBOLS.set(syms);
    let _ = API.set(api);
    lua::install_hooks()?;
    // (without it fse still works in single player; multiplayer then has no simulation events)
    if let Err(e) = mp::install_hooks() {
        log::line(&format!("multiplayer hooks: {e}"));
    }
    if std::env::var_os("FSE_LOADER").is_some() {
        // (installed in the game: a restarted game loads fse by itself)
        mods::sync();
    } else if let Err(e) = restart::install_hook() {
        // (a restart without it still works, just without fse in the new game)
        log::line(&format!("restart hook: {e}"));
    }
    mods::publish_paths();
    plugins::load_all();
    log::line(&format!("ready in {:.2?}", started.elapsed()));
    Ok(())
}

fn signal_ready(ok: bool) {
    let name: Vec<u16> = format!("Local\\fse-ready-{}", unsafe { GetCurrentProcessId() })
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
