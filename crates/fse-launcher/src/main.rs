//! fse launcher: starts factorio.exe suspended, loads fse.dll into it, waits until the DLL has read the
//! engine symbols and installed its hooks, then lets the game run. Exits with the game's exit code.
//!
//! Usage: fse [--game <path to factorio.exe>] [factorio arguments...]
//! The game path defaults to FACTORIO_EXE, else the Steam install. fse.dll is taken from beside this exe.
//! The log goes to FSE_LOG if set, else fse.log beside this exe.

#[path = "../../fse-core/src/inject.rs"]
mod inject;

use std::ffi::c_void;
use std::path::PathBuf;

use inject::{quote, wide};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, JOBOBJECT_BASIC_PROCESS_ID_LIST,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, OpenProcess, WaitForSingleObject, CREATE_SUSPENDED, INFINITE,
    PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, STARTUPINFOW,
};

/// the game: FACTORIO_EXE, else the first Steam library that has it (the usual places), else Steam's default
fn default_game() -> PathBuf {
    let rel = r"steamapps\common\Factorio\bin\x64\factorio.exe";
    let roots = [r"C:\Program Files (x86)\Steam", r"C:\Program Files\Steam", r"D:\SteamLibrary", r"E:\SteamLibrary",
                 r"F:\SteamLibrary", r"G:\SteamLibrary", r"D:\Steam", r"E:\Steam"];
    roots.iter().map(|r| PathBuf::from(r).join(rel)).find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(roots[0]).join(rel))
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut game = std::env::var("FACTORIO_EXE").map(PathBuf::from).unwrap_or_else(|_| default_game());
    if args.first().map(|a| a == "--game").unwrap_or(false) && args.len() > 1 {
        game = PathBuf::from(args.remove(1));
        args.remove(0);
    } else if args.first().map(|a| a.to_ascii_lowercase().ends_with("factorio.exe")).unwrap_or(false) {
        // as a Steam launch-option wrapper ("...\fse.exe" %COMMAND%): Steam passes the game first
        game = PathBuf::from(args.remove(0));
    }
    let here = std::env::current_exe().expect("own path");
    let dir = here.parent().expect("own dir").to_path_buf();
    let dll = dir.join("fse.dll");
    if !dll.exists() {
        eprintln!("fse: {} not found", dll.display());
        std::process::exit(2);
    }
    std::env::set_var("FSE_HOME", &dir);
    // fse.env beside the launcher: KEY=VALUE lines for the game process (FSE_PYPATH, FSE_PLUGINS, ...)
    if let Ok(text) = std::fs::read_to_string(dir.join("fse.env")) {
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if let Some((k, v)) = line.split_once('=') {
                std::env::set_var(k.trim(), v.trim());
            }
        }
    }
    if std::env::var_os("FSE_LOG").is_none() {
        std::env::set_var("FSE_LOG", dir.join("fse.log"));
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
            eprintln!("fse: starting {} failed ({})", game.display(), GetLastError());
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
        match inject::inject(&pi, dll) {
            Ok(true) => {}
            Ok(false) => eprintln!("fse: no ready signal from the DLL; the game runs without native support"),
            Err((code, msg)) => {
                eprintln!("fse: {msg}");
                return code;
            }
        }
        CloseHandle(pi.hThread);
        // a game restarting itself (after a mod change) starts the new game and exits: the new one is in the job
        // too, so wait on whatever is left there until the job is empty (closing the job would kill it)
        let mut game = pi.hProcess;
        let mut code = 0u32;
        while !game.is_null() {
            WaitForSingleObject(game, INFINITE);
            GetExitCodeProcess(game, &mut code);
            CloseHandle(game);
            game = if job.is_null() { std::ptr::null_mut() } else { next_in_job(job) };
        }
        code as i32
    }
}

/// a live process of the job, else null
unsafe fn next_in_job(job: *mut c_void) -> *mut c_void {
    #[repr(C)]
    struct List {
        head: JOBOBJECT_BASIC_PROCESS_ID_LIST,
        _more: [usize; 63],
    }
    let mut list: List = std::mem::zeroed();
    if QueryInformationJobObject(job, JobObjectBasicProcessIdList, &mut list as *mut _ as *mut c_void,
                                 std::mem::size_of::<List>() as u32, std::ptr::null_mut()) == 0 {
        return std::ptr::null_mut();
    }
    let ids = std::slice::from_raw_parts(list.head.ProcessIdList.as_ptr(), list.head.NumberOfProcessIdsInList as usize);
    for &id in ids {
        let h = OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, id as u32);
        if !h.is_null() {
            return h;
        }
    }
    std::ptr::null_mut()
}
