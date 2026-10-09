//! The pages in a panel over the game instead of the browser: a window owned by the game window (so it stays above
//! it, and goes with it), holding a WebView2 view of the same pages the web plugin serves. Made on first use and
//! hidden, not destroyed, when closed (Esc, or its close button), so it opens again at once. It lives on the overlay
//! thread, which pumps its messages: `open` from any thread posts there.
//!
//! WebView2 keeps its profile in <home>/webview (not beside factorio.exe). Without the WebView2 runtime, or with
//! FSE_PANEL=0, pages open in the browser as before.

use std::cell::RefCell;
use std::num::NonZeroIsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, CreateSolidBrush};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, IsWindowVisible, LoadCursorW, PostMessageW, RegisterClassW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_HWNDPARENT, IDC_ARROW, SWP_SHOWWINDOW,
    SW_HIDE, SW_SHOWNORMAL, WM_APP, WM_CLOSE, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

use crate::overlay::{game_window, wide};

const WM_OPEN: u32 = WM_APP + 1;
static HWND_PANEL: AtomicUsize = AtomicUsize::new(0);
static PENDING: Mutex<Option<String>> = Mutex::new(None);
static BASE: Mutex<String> = Mutex::new(String::new());

thread_local! {
    static VIEW: RefCell<Option<wry::WebView>> = const { RefCell::new(None) };
    static CONTEXT: RefCell<Option<wry::WebContext>> = const { RefCell::new(None) };
}

struct Parent(HWND);

impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let h = NonZeroIsize::new(self.0 as isize).ok_or(HandleError::Unavailable)?;
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(h))) })
    }
}

pub fn enabled() -> bool {
    std::env::var("FSE_PANEL").map(|v| v != "0").unwrap_or(true)
}

/// the web plugin's base address, ".../?token=..."; a page goes before the query
pub fn set_base(url: &str) {
    *BASE.lock().unwrap() = url.to_string();
}

fn page_url(path: &str) -> String {
    BASE.lock().unwrap().replacen("/?", &format!("/{path}?"), 1)
}

fn browser(url: &str) {
    let w = wide(url);
    unsafe {
        ShellExecuteW(std::ptr::null_mut(), wide("open").as_ptr(), w.as_ptr(), std::ptr::null(), std::ptr::null(),
                      SW_SHOWNORMAL);
    }
}

/// opens a page ("" the dashboard, "mods.html", ...): in the panel, else the browser. Any thread.
pub fn open(path: &str) {
    let url = page_url(path);
    let panel = HWND_PANEL.load(Ordering::Relaxed);
    if !enabled() || panel == 0 {
        return browser(&url);
    }
    *PENDING.lock().unwrap() = Some(url);
    unsafe { PostMessageW(panel as HWND, WM_OPEN, 0, 0) };
}

/// the panel: 80% of the game window's client area, centred on it
unsafe fn place(hwnd: HWND, game: HWND) {
    let mut r: RECT = std::mem::zeroed();
    GetClientRect(game, &mut r);
    let (gw, gh) = (r.right - r.left, r.bottom - r.top);
    let (w, h) = ((gw * 4 / 5).max(640), (gh * 4 / 5).max(420));
    let mut p = POINT { x: (gw - w) / 2, y: (gh - h) / 2 };
    ClientToScreen(game, &mut p);
    SetWindowPos(hwnd, std::ptr::null_mut(), p.x, p.y, w, h, SWP_SHOWWINDOW);
}

unsafe fn show(hwnd: HWND, url: String) {
    let game = game_window() as HWND;
    if !game.is_null() {
        SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, game as isize);
    }
    let made = VIEW.with(|v| {
        let mut v = v.borrow_mut();
        if let Some(view) = v.as_ref() {
            let _ = view.load_url(&url);
            return true;
        }
        let dir = std::env::var("FSE_HOME").map(std::path::PathBuf::from).unwrap_or_default().join("webview");
        CONTEXT.with(|c| {
            let mut c = c.borrow_mut();
            let ctx = c.get_or_insert_with(|| wry::WebContext::new(Some(dir)));
            let target = HWND_PANEL.load(Ordering::Relaxed) as usize;
            let built = wry::WebViewBuilder::new_with_web_context(ctx)
                .with_url(&url)
                .with_background_color((0x24, 0x24, 0x24, 0xff))
                // (Esc closes the panel, as Esc closes a game window)
                .with_initialization_script(
                    "addEventListener('keydown', e => { if (e.key === 'Escape') window.ipc.postMessage('close'); });")
                .with_ipc_handler(move |req| {
                    if req.body() == "close" {
                        PostMessageW(target as HWND, WM_CLOSE, 0, 0);
                    }
                })
                .build(&Parent(hwnd));
            match built {
                Ok(view) => {
                    *v = Some(view);
                    true
                }
                Err(e) => {
                    fse_plugin::log(&format!("panel: no WebView2 ({e}): pages open in the browser"));
                    false
                }
            }
        })
    });
    if !made {
        HWND_PANEL.store(0, Ordering::Relaxed); // (from now on the browser)
        return browser(&url);
    }
    if IsWindowVisible(hwnd) == 0 && !game.is_null() {
        place(hwnd, game);
    }
    ShowWindow(hwnd, SW_SHOWNORMAL);
    SetForegroundWindow(hwnd);
    VIEW.with(|v| v.borrow().as_ref().map(|view| view.focus()));
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_OPEN => {
            if let Some(url) = PENDING.lock().unwrap().take() {
                show(hwnd, url);
            }
            0
        }
        WM_CLOSE => {
            ShowWindow(hwnd, SW_HIDE);
            let game = game_window() as HWND;
            if !game.is_null() {
                SetForegroundWindow(game); // (back to the game, not whatever was behind)
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// makes the (hidden) panel window; on the overlay thread, before its message loop
pub unsafe fn create() {
    if !enabled() {
        return;
    }
    let class = wide("fse_panel");
    let inst = GetModuleHandleW(std::ptr::null());
    let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst, lpszClassName: class.as_ptr(),
                         hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                         hbrBackground: CreateSolidBrush(0x242424), ..std::mem::zeroed() };
    RegisterClassW(&wc);
    let title = wide("fse hub");
    let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW, 0, 0, 1024, 700,
                               game_window() as HWND, std::ptr::null_mut(), inst, std::ptr::null());
    if !hwnd.is_null() {
        // (a dark title bar, like the pages)
        let dark: i32 = 1;
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, &dark as *const i32 as *const _, 4);
        HWND_PANEL.store(hwnd as usize, Ordering::Relaxed);
    }
}
