//! fse loader: `version.dll` beside factorio.exe. Windows loads it from the game's own folder before the system one,
//! so dropping it into `bin\x64` is the whole install: no launcher, no Steam launch options, and deleting it uninstalls.
//! Every export forwards to the real `System32\version.dll` (build.rs).
//!
//! DllMain runs under the loader lock, too early to load fse there. It points the game's entry point at
//! `entry` instead; that runs once every DLL is initialised, loads `<game>\fse\fse.dll`, waits until fse has its hooks
//! in (the same ready event the launcher waits on), puts the entry point back and starts the game.
//!
//! Skipped when FSE_OFF is set (the test scripts' plain games) or when fse.dll is already in (the launcher put it there).

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{BOOL, HMODULE, TRUE};
use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE};
use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, GetCurrentProcessId, WaitForSingleObject,
};

type Entry = unsafe extern "system" fn(*mut c_void) -> u32;

// (written once in DllMain, read once in `entry`: both on the main thread)
static mut ORIGINAL: [u8; 14] = [0; 14];
static mut ENTRY: usize = 0;

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

unsafe fn write_code(at: usize, bytes: &[u8]) {
    let mut old = 0u32;
    VirtualProtect(at as *const c_void, bytes.len(), PAGE_EXECUTE_READWRITE, &mut old);
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), at as *mut u8, bytes.len());
    VirtualProtect(at as *const c_void, bytes.len(), old, &mut old);
    FlushInstructionCache(GetCurrentProcess(), at as *const c_void, bytes.len());
}

#[no_mangle]
pub extern "system" fn DllMain(_module: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
    let wanted = std::env::var_os("FSE_OFF").is_none() || std::env::var_os("FSE_NO_STEAM").is_some();
    if reason == DLL_PROCESS_ATTACH && wanted {
        unsafe {
            let base = GetModuleHandleW(std::ptr::null()) as usize;
            let nt = base + *((base + 0x3c) as *const u32) as usize;
            ENTRY = base + *((nt + 0x28) as *const u32) as usize; // (IMAGE_OPTIONAL_HEADER64.AddressOfEntryPoint)
            std::ptr::copy_nonoverlapping(ENTRY as *const u8, std::ptr::addr_of_mut!(ORIGINAL) as *mut u8, 14);
            // jmp [rip+0] ; <entry>
            let mut jmp = [0xFF, 0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            jmp[6..].copy_from_slice(&(entry as *const () as usize as u64).to_le_bytes());
            write_code(ENTRY, &jmp);
        }
    }
    TRUE
}

unsafe extern "system" fn entry(arg: *mut c_void) -> u32 {
    write_code(ENTRY, &*std::ptr::addr_of!(ORIGINAL));
    if std::env::var_os("FSE_NO_STEAM").is_some() {
        no_steam();
    }
    if std::env::var_os("FSE_OFF").is_none() {
        load();
    }
    let original: Entry = std::mem::transmute(ENTRY);
    original(arg)
}

/// FSE_NO_STEAM (the test scripts): the game starts as if Steam weren't there. A Steam-started game keeps the
/// player's settings (player-data.json: the shortcut bar...) in Steam's own folder, which every game started under
/// that account shares, whatever its write-data: a test game with other mods would rewrite them. Steam's
/// SteamAPI_RestartAppIfNecessary and SteamAPI_Init answer false; the game then runs like the standalone one.
unsafe fn no_steam() {
    let steam = GetModuleHandleW(wide("steam_api64.dll".as_ref()).as_ptr());
    if steam.is_null() {
        return;
    }
    for name in [c"SteamAPI_Init", c"SteamAPI_RestartAppIfNecessary"] {
        if let Some(f) = windows_sys::Win32::System::LibraryLoader::GetProcAddress(steam, name.as_ptr() as *const u8) {
            write_code(f as usize, &[0x31, 0xC0, 0xC3]); // xor eax, eax ; ret
        }
    }
}

/// `<game>\fse`: two folders up from `bin\x64\factorio.exe`
fn home() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.parent()?.join("fse"))
}

unsafe fn load() {
    if !GetModuleHandleW(wide("fse.dll".as_ref()).as_ptr()).is_null() {
        return;
    }
    let Some(home) = home() else { return };
    let dll = home.join("fse.dll");
    if !dll.exists() {
        return;
    }
    set_env(&home);
    let ev = CreateEventW(std::ptr::null(), 1, 0,
                          wide(format!("Local\\fse-ready-{}", GetCurrentProcessId()).as_ref()).as_ptr());
    if LoadLibraryW(wide(dll.as_os_str()).as_ptr()).is_null() {
        return;
    }
    // (reading the game's pdb takes a few seconds on a new build; past this the game starts without fse's hooks)
    WaitForSingleObject(ev, 120_000);
}

/// what the launcher sets: FSE_HOME, FSE_LOG and fse.env's KEY=value lines
fn set_env(home: &Path) {
    std::env::set_var("FSE_HOME", home);
    std::env::set_var("FSE_LOADER", "1");
    if std::env::var_os("FSE_LOG").is_none() {
        std::env::set_var("FSE_LOG", home.join("fse.log"));
    }
    if let Ok(text) = std::fs::read_to_string(home.join("fse.env")) {
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if let Some((k, v)) = line.split_once('=') {
                std::env::set_var(k.trim(), v.trim());
            }
        }
    }
}
