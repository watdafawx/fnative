//! fnative launcher: starts factorio.exe suspended, loads fnative.dll into it, waits until the DLL has read the
//! engine symbols and installed its hooks, then lets the game run. Exits with the game's exit code.
//!
//! Usage: factorio-native [--game <path to factorio.exe>] [factorio arguments...]
//! The game path defaults to FACTORIO_EXE, else the Steam install. fnative.dll is taken from beside this exe.
//! The log goes to FNATIVE_LOG if set, else fnative.log beside this exe.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{VirtualAllocEx, MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateProcessW, CreateRemoteThread, GetExitCodeProcess, GetExitCodeThread, ResumeThread,
    TerminateProcess, WaitForSingleObject, CREATE_SUSPENDED, INFINITE, PROCESS_INFORMATION, STARTUPINFOW,
};

/// the game: FACTORIO_EXE, else the first Steam library that has it (the usual places), else Steam's default
fn default_game() -> PathBuf {
    let rel = r"steamapps\common\Factorio\bin\x64\factorio.exe";
    let roots = [r"C:\Program Files (x86)\Steam", r"C:\Program Files\Steam", r"D:\SteamLibrary", r"E:\SteamLibrary",
                 r"F:\SteamLibrary", r"G:\SteamLibrary", r"D:\Steam", r"E:\Steam"];
    roots.iter().map(|r| PathBuf::from(r).join(rel)).find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(roots[0]).join(rel))
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

fn quote(arg: &str) -> String {
    if arg.is_empty() || arg.contains([' ', '\t', '"']) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_string()
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut game = std::env::var("FACTORIO_EXE").map(PathBuf::from).unwrap_or_else(|_| default_game());
    if args.first().map(|a| a == "--game").unwrap_or(false) && args.len() > 1 {
        game = PathBuf::from(args.remove(1));
        args.remove(0);
    } else if args.first().map(|a| a.to_ascii_lowercase().ends_with("factorio.exe")).unwrap_or(false) {
        // as a Steam launch-option wrapper ("...\factorio-native.exe" %COMMAND%): Steam passes the game first
        game = PathBuf::from(args.remove(0));
    }
    let here = std::env::current_exe().expect("own path");
    let dir = here.parent().expect("own dir").to_path_buf();
    let dll = dir.join("fnative.dll");
    if !dll.exists() {
        eprintln!("fnative: {} not found", dll.display());
        std::process::exit(2);
    }
    std::env::set_var("FNATIVE_HOME", &dir);
    // fnative.env beside the launcher: KEY=VALUE lines for the game process (FNATIVE_PYPATH, FNATIVE_PLUGINS, ...)
    if let Ok(text) = std::fs::read_to_string(dir.join("fnative.env")) {
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if let Some((k, v)) = line.split_once('=') {
                std::env::set_var(k.trim(), v.trim());
            }
        }
    }
    if std::env::var_os("FNATIVE_LOG").is_none() {
        std::env::set_var("FNATIVE_LOG", dir.join("fnative.log"));
    }
    // Steam restarts a directly started game (dropping its arguments) unless it looks Steam-launched
    std::env::set_var("SteamAppId", "427520");
    std::env::set_var("SteamGameId", "427520");
    std::process::exit(run(&game, &args, &dll));
}

fn run(game: &PathBuf, args: &[String], dll: &PathBuf) -> i32 {
    let mut cmd = quote(&game.to_string_lossy());
    for a in args {
        cmd.push(' ');
        cmd.push_str(&quote(a));
    }
    let mut cmdw = wide(std::ffi::OsStr::new(&cmd));
    unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        // (no working directory of its own: the caller's, so relative arguments keep working)
        if CreateProcessW(std::ptr::null(), cmdw.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 1,
                          CREATE_SUSPENDED, std::ptr::null(), std::ptr::null(), &si, &mut pi) == 0 {
            eprintln!("fnative: starting {} failed ({})", game.display(), GetLastError());
            return 3;
        }
        // the game lives in a job that ends with this launcher: killing the launcher (or Steam stopping it) ends the
        // game too, instead of leaving it running unseen
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if !job.is_null() {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const c_void,
                                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
            AssignProcessToJobObject(job, pi.hProcess);
        }
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
        if thread.is_null() {
            eprintln!("fnative: injecting failed ({})", GetLastError());
            TerminateProcess(pi.hProcess, 1);
            return 4;
        }
        WaitForSingleObject(thread, INFINITE);
        let mut loaded = 0u32;
        GetExitCodeThread(thread, &mut loaded);
        CloseHandle(thread);
        if loaded == 0 {
            eprintln!("fnative: the game could not load {}", dll.display());
            TerminateProcess(pi.hProcess, 1);
            return 5;
        }
        // reading a 400 MB pdb takes a few seconds; give it plenty before running the game without native support
        if WaitForSingleObject(ready, 120_000) != WAIT_OBJECT_0 {
            eprintln!("fnative: no ready signal from the DLL; the game runs without native support");
        }
        ResumeThread(pi.hThread);
        WaitForSingleObject(pi.hProcess, INFINITE);
        let mut code = 0u32;
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        code as i32
    }
}
