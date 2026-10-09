//! The buttons over the game window's top-left corner: the way into the hub from the main menu, where no mod can add
//! anything (the menu is the engine's own GUI). A small popup window owned by the game window (so it stays above it),
//! shown only while the game window is in front and no game is ticking (the main menu, or the pause menu); in a
//! running game the fse-hub mod's button takes over.
//!
//!   [ fse hub ][ started in 1 min 31 s ]
//! The first opens the dashboard, the second the startup report (startup.html), in the panel (panel.rs). The startup
//! time is read from the game's log ("Factorio initialised", seconds since the game started) once it appears.
//! Either button can be turned off from the dashboard (overlay.json beside the launcher: {"hub": .., "startup": ..}).
//! The thread here also runs the panel's window, with or without the buttons.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    InvalidateRect, Rectangle, SelectObject, SetBkMode, SetTextColor, DT_CENTER, DT_SINGLELINE, DT_VCENTER, PAINTSTRUCT,
    PS_SOLID, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, EnumWindows, GetClientRect, GetForegroundWindow, GetMessageW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, LoadCursorW, RegisterClassW, SetTimer,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, GWLP_HWNDPARENT, IDC_HAND, MA_NOACTIVATE, MSG,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_PAINT,
    WM_TIMER, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

static GAME: AtomicUsize = AtomicUsize::new(0);
static HOVER: AtomicI32 = AtomicI32::new(-1); // -1 none, 0 the hub button, 1 the startup button
static IN_GAME: OnceLock<fn() -> bool> = OnceLock::new();
static STARTUP: Mutex<Option<f64>> = Mutex::new(None);
static STARTED_AT: OnceLock<std::time::SystemTime> = OnceLock::new();
static TICKS: AtomicUsize = AtomicUsize::new(0);
static SHOW_HUB: AtomicBool = AtomicBool::new(true);
static SHOW_STARTUP: AtomicBool = AtomicBool::new(true);
const WM_MOUSELEAVE: u32 = 0x02A3;
const HUB_W: i32 = 112;
const START_W: i32 = 190;
const H: i32 = 30;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

fn settings_path() -> std::path::PathBuf {
    crate::home().join("overlay.json")
}

/// {"hub": bool, "startup": bool}: which buttons show
pub fn settings() -> serde_json::Value {
    serde_json::json!({"hub": SHOW_HUB.load(Ordering::Relaxed), "startup": SHOW_STARTUP.load(Ordering::Relaxed)})
}

/// takes the keys given in `v` (the others stay), saves them for the next start
pub fn set(v: &serde_json::Value) -> std::io::Result<()> {
    if let Some(b) = v.get("hub").and_then(serde_json::Value::as_bool) {
        SHOW_HUB.store(b, Ordering::Relaxed);
    }
    if let Some(b) = v.get("startup").and_then(serde_json::Value::as_bool) {
        SHOW_STARTUP.store(b, Ordering::Relaxed);
    }
    std::fs::write(settings_path(), settings().to_string())
}

/// the buttons showing now, left to right: (0 hub / 1 startup, x, width)
fn segments() -> Vec<(i32, i32, i32)> {
    let mut v = Vec::new();
    let mut x = 0;
    if SHOW_HUB.load(Ordering::Relaxed) {
        v.push((0, x, HUB_W));
        x += HUB_W - 2;
    }
    if SHOW_STARTUP.load(Ordering::Relaxed) && STARTUP.lock().unwrap().is_some() {
        v.push((1, x, START_W + 2));
    }
    v
}

fn width() -> i32 {
    segments().last().map(|&(_, x, w)| x + w).unwrap_or(0)
}

fn segment_at(x: i32) -> i32 {
    segments().iter().find(|&&(_, sx, w)| x >= sx && x < sx + w).map(|s| s.0).unwrap_or(-1)
}

fn fmt_secs(s: f64) -> String {
    if s >= 60.0 { format!("{} min {} s", (s / 60.0) as u32, (s % 60.0).round() as u32) } else { format!("{s:.1} s") }
}

/// the game's log: FACTORIO_LOG, else beside the mods folder (FACTORIO_MODS)
fn log_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("FACTORIO_LOG") {
        return p.into();
    }
    let mods = std::env::var("FACTORIO_MODS").unwrap_or_else(|_| {
        // (the game's default place: %APPDATA%\Factorio\mods)
        format!(r"{}\Factorio\mods", std::env::var("APPDATA").unwrap_or_default())
    });
    std::path::Path::new(&mods).parent().map(|p| p.join("factorio-current.log")).unwrap_or_default()
}

/// "  90.702 Factorio initialised" in this run's log (not an older one: it must be newer than this process)
fn read_startup() -> Option<f64> {
    let p = log_path();
    let meta = std::fs::metadata(&p).ok()?;
    let started = *STARTED_AT.get()?;
    if meta.modified().ok()? + std::time::Duration::from_secs(5) < started {
        return None;
    }
    let text = std::fs::read_to_string(&p).ok()?;
    let line = text.lines().find(|l| l.contains("Factorio initialised"))?;
    line.split_whitespace().next()?.parse().ok()
}

/// the game window as last found (0 before it exists)
pub fn game_window() -> usize {
    GAME.load(Ordering::Relaxed)
}

/// the game's main window: the biggest visible top-level window of this process (0 until it exists)
fn find_game() -> usize {
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
            // (our own buttons are small: the game window is far bigger)
            if area > best.area && area > 200 * 200 {
                best.hwnd = hwnd as usize;
                best.area = area;
            }
        }
        1
    }
    let mut best = Best { hwnd: 0, area: 0 };
    unsafe { EnumWindows(Some(each), &mut best as *mut Best as LPARAM) };
    best.hwnd
}

unsafe fn paint_segment(dc: *mut core::ffi::c_void, x: i32, w: i32, text: &str, hover: bool) {
    let bg = CreateSolidBrush(if hover { rgb(0xf0, 0xa2, 0x30) } else { rgb(0x31, 0x33, 0x35) });
    let pen = CreatePen(PS_SOLID, 2, rgb(0x00, 0x00, 0x00));
    let old_b = SelectObject(dc, bg);
    let old_p = SelectObject(dc, pen);
    Rectangle(dc, x, 0, x + w, H);
    SelectObject(dc, old_b);
    SelectObject(dc, old_p);
    if !hover {
        let edge = CreateSolidBrush(rgb(0xf0, 0xa2, 0x30));
        let strip = RECT { left: x + 2, top: H - 4, right: x + w - 2, bottom: H - 2 };
        FillRect(dc, &strip, edge);
        DeleteObject(edge);
    }
    let font = CreateFontW(17, 0, 0, 0, 700, 0, 0, 0, 0, 0, 0, 5, 0, wide("Segoe UI").as_ptr());
    let old_f = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, if hover { rgb(0x1d, 0x1f, 0x21) } else { rgb(0xff, 0xc8, 0x64) });
    let mut r = RECT { left: x, top: 0, right: x + w, bottom: H };
    let mut t = wide(text);
    DrawTextW(dc, t.as_mut_ptr(), -1, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    SelectObject(dc, old_f);
    DeleteObject(font);
    DeleteObject(bg);
    DeleteObject(pen);
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            // (until the game has started: look for the startup time in its log about once a second)
            let n = TICKS.fetch_add(1, Ordering::Relaxed);
            if STARTUP.lock().unwrap().is_none() && n % 4 == 0 {
                if let Some(s) = read_startup() {
                    *STARTUP.lock().unwrap() = Some(s);
                    InvalidateRect(hwnd, std::ptr::null(), 1);
                }
            }
            // (the game makes its window again after loading, and for fullscreen switches: follow the current one)
            let mut game = GAME.load(Ordering::Relaxed) as HWND;
            let now = find_game() as HWND;
            if !now.is_null() && now != game {
                game = now;
                GAME.store(now as usize, Ordering::Relaxed);
                SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, now as isize);
            }
            if IsWindow(game) == 0 {
                ShowWindow(hwnd, SW_HIDE);
                return 0;
            }
            let fg = GetForegroundWindow();
            let w = width();
            let show = w > 0 && IsWindowVisible(game) != 0 && IsIconic(game) == 0 && (fg == game || fg == hwnd)
                && !IN_GAME.get().map(|f| f()).unwrap_or(false);
            if show {
                let mut p = POINT { x: 12, y: 12 };
                ClientToScreen(game, &mut p);
                SetWindowPos(hwnd, std::ptr::null_mut(), p.x, p.y, w, H, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            } else {
                ShowWindow(hwnd, SW_HIDE);
            }
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let hover = HOVER.load(Ordering::Relaxed);
            for (kind, x, w) in segments() {
                let text = if kind == 0 { "fse  hub".to_string() }
                           else { format!("started in {}", fmt_secs(STARTUP.lock().unwrap().unwrap_or(0.0))) };
                paint_segment(dc, x, w, &text, hover == kind);
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_MOUSEMOVE => {
            let x = (lp & 0xffff) as i16 as i32;
            let seg = segment_at(x);
            if HOVER.swap(seg, Ordering::Relaxed) != seg {
                let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE,
                                              hwndTrack: hwnd, dwHoverTime: 0 };
                TrackMouseEvent(&mut t);
                InvalidateRect(hwnd, std::ptr::null(), 1);
            }
            0
        }
        WM_MOUSELEAVE => {
            HOVER.store(-1, Ordering::Relaxed);
            InvalidateRect(hwnd, std::ptr::null(), 1);
            0
        }
        WM_LBUTTONUP => {
            let x = (lp & 0xffff) as i16 as i32;
            match segment_at(x) {
                0 => crate::panel::open(""),
                1 => crate::panel::open("startup.html"),
                _ => {}
            }
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT, // (a click must not take focus from the game)
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// shows the buttons (if `buttons`) for the game window, once it exists, and makes the panel's window; `in_game`
/// says a game is ticking (then the buttons hide)
pub fn start(buttons: bool, in_game: fn() -> bool) {
    let _ = IN_GAME.set(in_game);
    let _ = STARTED_AT.set(std::time::SystemTime::now());
    if let Some(v) = std::fs::read_to_string(settings_path()).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        let _ = set(&v);
    }
    std::thread::Builder::new().name("fse-overlay".into()).spawn(move || unsafe {
        let game = loop {
            let g = find_game();
            if g != 0 {
                break g;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        };
        GAME.store(game, Ordering::Relaxed);
        crate::panel::create();
        if !buttons {
            // (the panel alone: keep pumping its messages)
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            return;
        }
        let class = wide("fse_overlay");
        let inst = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst, lpszClassName: class.as_ptr(),
                             hCursor: LoadCursorW(std::ptr::null_mut(), IDC_HAND), ..std::mem::zeroed() };
        RegisterClassW(&wc);
        let title = wide("fse");
        let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, class.as_ptr(), title.as_ptr(), WS_POPUP, 0, 0,
                                   HUB_W, H, game as HWND, std::ptr::null_mut(), inst, std::ptr::null());
        if hwnd.is_null() {
            fse_plugin::log("overlay: could not create its window");
            return;
        }
        SetTimer(hwnd, 1, 250, None);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }).ok();
}
