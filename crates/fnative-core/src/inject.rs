//! Loads fnative.dll into a game started suspended, waits until the DLL has its hooks in, then resumes the game.
//! Shared by the launcher and by the DLL itself (a game restarting itself after a mod change starts the new game
//! through it): the launcher includes this file with `#[path]`.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{VirtualAllocEx, MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateRemoteThread, GetExitCodeThread, ResumeThread, TerminateProcess, WaitForSingleObject,
    INFINITE, PROCESS_INFORMATION,
};

pub fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

pub fn quote(arg: &str) -> String {
    if arg.is_empty() || arg.contains([' ', '\t', '"']) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_string()
    }
}

/// `pi` is a game started with CREATE_SUSPENDED. Ok(false): the DLL never said it was ready, the game runs without
/// native support. Err: the game was terminated, with an exit code and why.
pub unsafe fn inject(pi: &PROCESS_INFORMATION, dll: &Path) -> Result<bool, (i32, String)> {
    // the DLL signals this event when its hooks are in (or when it gave up: the game then runs without them)
    let ev_name = wide(std::ffi::OsStr::new(&format!("Local\\fnative-ready-{}", pi.dwProcessId)));
    let ready = CreateEventW(std::ptr::null(), 1, 0, ev_name.as_ptr());

    let path = wide(dll.as_os_str());
    let bytes = path.len() * 2;
    let remote = VirtualAllocEx(pi.hProcess, std::ptr::null(), bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    let mut written = 0usize;
    let ok = !remote.is_null()
        && WriteProcessMemory(pi.hProcess, remote, path.as_ptr() as *const c_void, bytes, &mut written) != 0;
    // kernel32 sits at the same address in every process of a session
    let k32 = GetModuleHandleW(wide(std::ffi::OsStr::new("kernel32.dll")).as_ptr());
    let load = GetProcAddress(k32, c"LoadLibraryW".as_ptr() as *const u8);
    let thread = if ok {
        CreateRemoteThread(pi.hProcess, std::ptr::null(), 0, std::mem::transmute(load), remote, 0,
                           std::ptr::null_mut())
    } else {
        std::ptr::null_mut()
    };
    let fail = |code: i32, msg: String| {
        TerminateProcess(pi.hProcess, 1);
        CloseHandle(ready);
        Err((code, msg))
    };
    if thread.is_null() {
        return fail(4, format!("injecting failed ({})", GetLastError()));
    }
    WaitForSingleObject(thread, INFINITE);
    let mut loaded = 0u32;
    GetExitCodeThread(thread, &mut loaded);
    CloseHandle(thread);
    if loaded == 0 {
        return fail(5, format!("the game could not load {}", dll.display()));
    }
    // reading a 400 MB pdb takes a few seconds; give it plenty before running the game without native support
    let signalled = WaitForSingleObject(ready, 120_000) == WAIT_OBJECT_0;
    CloseHandle(ready);
    ResumeThread(pi.hThread);
    Ok(signalled)
}
