//! The game restarts itself (after a mod change, "sync mods with save", ...) by ShellExecuteW-ing factorio.exe with
//! its reload arguments, then exiting. That new game would run without fse: start it here instead, suspended,
//! and load fse into it the way the launcher does. (It stays in the launcher's job, and the launcher waits until
//! the job is empty.)

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::OnceLock;

use retour::GenericDetour;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, CREATE_SUSPENDED, PROCESS_INFORMATION, STARTUPINFOW,
};
use windows_sys::Win32::Foundation::CloseHandle;

use crate::inject::{inject, quote, wide};
use crate::log;

type ShellExecuteFn = unsafe extern "system" fn(*mut c_void, *const u16, *const u16, *const u16, *const u16, i32)
    -> *mut c_void;
static SHELL_EXECUTE: OnceLock<GenericDetour<ShellExecuteFn>> = OnceLock::new();

unsafe fn text(p: *const u16) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let n = (0..).take_while(|&i| *p.add(i) != 0).count();
    Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)))
}

unsafe extern "system" fn shell_execute(hwnd: *mut c_void, op: *const u16, file: *const u16, params: *const u16,
                                        dir: *const u16, show: i32) -> *mut c_void {
    let original = SHELL_EXECUTE.get().expect("detour");
    let file_s = text(file).unwrap_or_default();
    let is_game = PathBuf::from(&file_s).file_name()
        .map(|n| n.to_string_lossy().eq_ignore_ascii_case("factorio.exe")).unwrap_or(false);
    let is_open = text(op).map(|o| o.eq_ignore_ascii_case("open")).unwrap_or(true);
    if is_game && is_open {
        if let Some(home) = std::env::var_os("FSE_HOME") {
            let args = text(params).unwrap_or_default();
            log::line(&format!("the game restarts itself: {file_s} {args}"));
            if start(&file_s, &args, dir, &PathBuf::from(home).join("fse.dll")) {
                return 42 as *mut c_void; // (ShellExecuteW: above 32 is success)
            }
            log::line("the restarted game runs without native support");
        }
    }
    original.call(hwnd, op, file, params, dir, show)
}

/// the new game, suspended, with fse loaded; false: it isn't running (the caller starts it the plain way)
unsafe fn start(file: &str, args: &str, dir: *const u16, dll: &std::path::Path) -> bool {
    let cmd = if args.is_empty() { quote(file) } else { format!("{} {args}", quote(file)) };
    let mut cmdw = wide(std::ffi::OsStr::new(&cmd));
    let mut si: STARTUPINFOW = std::mem::zeroed();
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
    if CreateProcessW(std::ptr::null(), cmdw.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0, CREATE_SUSPENDED,
                      std::ptr::null(), dir, &si, &mut pi) == 0 {
        log::line(&format!("starting the restarted game failed ({})",
                           windows_sys::Win32::Foundation::GetLastError()));
        return false;
    }
    // (synchronous on purpose: this game exits right after, and would take a half-done injection with it)
    let ok = match inject(&pi, dll) {
        Ok(ready) => {
            log::line(if ready { "restarted game is ready" } else { "no ready signal from the restarted game" });
            true
        }
        Err((_, msg)) => {
            log::line(&msg);
            false
        }
    };
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    ok
}

pub fn install_hook() -> Result<(), String> {
    unsafe {
        let shell32 = LoadLibraryW(wide(std::ffi::OsStr::new("shell32.dll")).as_ptr());
        let target = GetProcAddress(shell32, c"ShellExecuteW".as_ptr() as *const u8).ok_or("no ShellExecuteW")?;
        let d = GenericDetour::<ShellExecuteFn>::new(std::mem::transmute(target), shell_execute)
            .map_err(|e| format!("ShellExecuteW detour: {e}"))?;
        d.enable().map_err(|e| format!("ShellExecuteW detour enable: {e}"))?;
        let _ = SHELL_EXECUTE.set(d);
    }
    Ok(())
}
