//! fnative plugin "std": what the Lua API can't reach, for mods and the fnative-std Lua library.
//!
//!   input        {"x","y": cursor in the game window's client area (pixels, may be outside), "inside", "focused",
//!                 "left","right","middle": buttons held, "shift","ctrl","alt": keys held, "w","h": client size,
//!                 "wheel": mouse wheel notches turned over the focused game window since the game started (up +,
//!                 down -): a running total, so each reader keeps its own last value}
//!                (buttons and keys only count while the game window has focus)
//!   clipboard_get / clipboard_set(text)
//!   now          milliseconds since 1970 (real time; Lua in the game has no clock)
//!   read(path)   a file under script-output (no "..", no absolute paths): Lua can write files, not read them
//!   open(url)    opens an http(s) address in the browser
//!   wheel_capture(ms)  keep the wheel from the game for the next ms (max 1000; "0" ends it): renew it while the
//!                cursor is over a view the mod zooms, so the map behind doesn't zoom too
//!   mock(json)   tests: fields given here replace the real input until mock("") (e.g. {"left": true, "x": 100})
//! All threadsafe. Single player: input differs per machine, so game state driven by it would desync multiplayer.

use std::sync::Mutex;

use fnative_plugin as fp;
use serde_json::{json, Map, Value};
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_LBUTTON, VK_MBUTTON, VK_MENU, VK_RBUTTON, VK_SHIFT};
use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClientRect, GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId, IsWindow, IsWindowVisible};

static MOCK: Mutex<Option<Map<String, Value>>> = Mutex::new(None);
static WHEEL: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
// wheel_capture: until this moment (ms since 1970) the wheel over the game window is kept from the game (counted
// here only): a mod's view under the cursor zooms, the map behind doesn't. A lease, renewed while wanted, so a mod
// that stops asking (a window closed, the game paused) gives the wheel back by itself within a quarter second.
static CAPTURE_UNTIL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

// the wheel: no API reads it, so a low-level mouse hook (its own thread and message loop) counts the notches turned
// while the game window is in front; it only reads, and hands every event on unchanged
unsafe extern "system" fn mouse_hook(code: i32, wp: usize, lp: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{CallNextHookEx, MSLLHOOKSTRUCT, WM_MOUSEWHEEL};
    if code >= 0 && wp as u32 == WM_MOUSEWHEEL {
        let info = &*(lp as *const MSLLHOOKSTRUCT);
        let hwnd = *WINDOW.lock().unwrap() as HWND;
        if !hwnd.is_null() && GetForegroundWindow() == hwnd {
            let delta = (info.mouseData >> 16) as i16 as i64;
            WHEEL.fetch_add(delta, std::sync::atomic::Ordering::Relaxed);
            if now_ms() < CAPTURE_UNTIL.load(std::sync::atomic::Ordering::Relaxed) {
                return 1; // (kept from the game)
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wp, lp)
}

fn start_wheel() {
    std::thread::Builder::new().name("fnative-std-wheel".into()).spawn(|| unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage, MSG, WH_MOUSE_LL};
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), std::ptr::null_mut(), 0);
        if hook.is_null() {
            fp::log("no mouse wheel (the hook was refused)");
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }).ok();
}
static WINDOW: Mutex<usize> = Mutex::new(0);

/// the game's main window: the biggest visible top-level window of this process
fn game_window() -> HWND {
    let mut w = WINDOW.lock().unwrap();
    if *w != 0 && unsafe { IsWindow(*w as HWND) } != 0 {
        return *w as HWND;
    }
    struct Best {
        hwnd: usize,
        area: i64,
    }
    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
        let best = &mut *(lp as *mut Best);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 {
            let mut r: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut r);
            let area = (r.right - r.left) as i64 * (r.bottom - r.top) as i64;
            if area > best.area {
                best.hwnd = hwnd as usize;
                best.area = area;
            }
        }
        1
    }
    let mut best = Best { hwnd: 0, area: 0 };
    unsafe { EnumWindows(Some(each), &mut best as *mut Best as LPARAM) };
    *w = best.hwnd;
    best.hwnd as HWND
}

fn held(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

fn input() -> Value {
    let hwnd = game_window();
    let mut v = Map::new();
    unsafe {
        let mut p = POINT { x: 0, y: 0 };
        GetCursorPos(&mut p);
        let mut r: RECT = std::mem::zeroed();
        if !hwnd.is_null() {
            ScreenToClient(hwnd, &mut p);
            GetClientRect(hwnd, &mut r);
        }
        let focused = !hwnd.is_null() && GetForegroundWindow() == hwnd;
        let inside = p.x >= 0 && p.y >= 0 && p.x < r.right && p.y < r.bottom;
        v.insert("x".into(), json!(p.x));
        v.insert("y".into(), json!(p.y));
        v.insert("w".into(), json!(r.right));
        v.insert("h".into(), json!(r.bottom));
        v.insert("inside".into(), json!(inside));
        v.insert("focused".into(), json!(focused));
        v.insert("wheel".into(), json!(WHEEL.load(std::sync::atomic::Ordering::Relaxed) / 120));
        for (name, vk) in [("left", VK_LBUTTON), ("right", VK_RBUTTON), ("middle", VK_MBUTTON), ("shift", VK_SHIFT),
                           ("ctrl", VK_CONTROL), ("alt", VK_MENU)] {
            v.insert(name.into(), json!(focused && held(vk)));
        }
    }
    if let Some(m) = MOCK.lock().unwrap().as_ref() {
        for (k, x) in m {
            v.insert(k.clone(), x.clone());
        }
    }
    Value::Object(v)
}

// wheel_capture "250": keep the wheel from the game for the next 250 ms (at most 1000), "0": give it back now
fp::export!(f_wheel_capture, |_, input| {
    let ms: u64 = input.trim().parse().unwrap_or(0).min(1000);
    CAPTURE_UNTIL.store(if ms == 0 { 0 } else { now_ms() + ms }, std::sync::atomic::Ordering::Relaxed);
    Ok("ok".into())
});

fn clipboard_get() -> Result<String, String> {
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err("clipboard busy".into());
        }
        let h = GetClipboardData(13); // (CF_UNICODETEXT)
        let mut out = String::new();
        if !h.is_null() {
            let p = GlobalLock(h) as *const u16;
            if !p.is_null() {
                let mut n = 0;
                while *p.add(n) != 0 {
                    n += 1;
                }
                out = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                GlobalUnlock(h);
            }
        }
        CloseClipboard();
        Ok(out)
    }
}

fn clipboard_set(text: &str) -> Result<String, String> {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err("clipboard busy".into());
        }
        EmptyClipboard();
        let h = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if h.is_null() {
            CloseClipboard();
            return Err("out of memory".into());
        }
        let p = GlobalLock(h) as *mut u16;
        std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
        GlobalUnlock(h);
        SetClipboardData(13, h);
        CloseClipboard();
    }
    Ok("ok".into())
}

/// script-output: FACTORIO_SCRIPT_OUTPUT, else beside the mods folder (FACTORIO_MODS)
fn script_output() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("FACTORIO_SCRIPT_OUTPUT") {
        return p.into();
    }
    let mods = std::env::var("FACTORIO_MODS").unwrap_or_else(|_| {
        // (the game's default place: %APPDATA%\Factorio\mods)
        format!(r"{}\Factorio\mods", std::env::var("APPDATA").unwrap_or_default())
    });
    std::path::Path::new(&mods).parent().map(|p| p.join("script-output")).unwrap_or_default()
}

fn read(path: &str) -> Result<String, String> {
    let p = std::path::Path::new(path);
    if p.is_absolute() || path.contains("..") || path.contains(':') {
        return Err("read: only paths inside script-output".into());
    }
    std::fs::read_to_string(script_output().join(p)).map_err(|e| format!("read {path}: {e}"))
}

fp::export!(f_input, |_, _| Ok(input().to_string()));
fp::export!(f_clip_get, |_, _| clipboard_get());
fp::export!(f_clip_set, |_, text| clipboard_set(text));
fp::export!(f_now, |_, _| {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);
    Ok(format!("{t:.3}"))
});
fp::export!(f_read, |_, path| read(path.trim()));
fp::export!(f_open, |_, url| {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("open: only http(s) addresses".into());
    }
    let w: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), w.as_ptr(), std::ptr::null(),
                                                     std::ptr::null(), windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
    }
    Ok("opened".into())
});
fp::export!(f_mock, |_, text| {
    let mut m = MOCK.lock().unwrap();
    if text.trim().is_empty() {
        *m = None;
        return Ok("real input".into());
    }
    let v: Value = serde_json::from_str(text).map_err(|e| format!("mock: {e}"))?;
    let obj = v.as_object().cloned().ok_or("mock: give a JSON object")?;
    let merged = m.get_or_insert_with(Map::new);
    for (k, x) in obj {
        merged.insert(k, x);
    }
    Ok("mocked".into())
});

#[no_mangle]
pub unsafe extern "C" fn fnative_plugin_init(host: *const fp::Host) -> i32 {
    if !fp::init(host, "std") {
        return 1;
    }
    for (f, n) in [(f_input as fp::PluginFn, "input"), (f_clip_get, "clipboard_get"), (f_clip_set, "clipboard_set"),
                   (f_now, "now"), (f_read, "read"), (f_mock, "mock"), (f_open, "open"),
                   (f_wheel_capture, "wheel_capture")] {
        fp::register("std", n, f, fp::THREADSAFE);
    }
    if std::env::var("FNATIVE_STD_WHEEL").map(|v| v != "0").unwrap_or(true) {
        start_wheel();
    }
    0
}
