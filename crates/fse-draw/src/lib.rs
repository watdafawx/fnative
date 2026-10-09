//! fse plugin "draw": lines, rectangles, circles and text drawn over the game world every frame, on this computer only
//! (no game state: safe in multiplayer, and for things only this player should see). Nothing in the game's renderer
//! is touched: a transparent window that lets clicks through lies over the game window's client area, and each frame
//! the shapes are drawn into it where the camera (GameView's cached surface view: position, zoom) puts them. It sits
//! above the game's own GUI too.
//!
//!   draw.set    {"id": "my-mod:route", "surface": 1, "shapes": [...]}   (a layer: set again to change it)
//!                 {"line": [[x1, y1], [x2, y2]], "color": [r, g, b, a], "width": 2}
//!                 {"rect": [[x1, y1], [x2, y2]], "color": [...], "width": 2}      (width 0: filled)
//!                 {"circle": [x, y], "radius": 3, "color": [...], "width": 2}     (radius in tiles; width 0: filled)
//!                 {"text": "hello", "at": [x, y], "size": 16, "color": [...]}
//!               positions in tiles (map coordinates), widths and text size in screen pixels, colours 0..1 (a: 1 by
//!               default); surface: the surface's index as Lua has it (default: any)
//!   draw.clear  {"id": ...} (no id: every layer)
//!   draw.status {layers, shapes, frames, camera}
//! Not over an exclusive fullscreen game (only a window, or borderless).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};

use fse_plugin as fp;
use serde_json::{json, Value};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, DIB_RGB_COLORS,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

static LAYERS: Mutex<BTreeMap<String, Layer>> = Mutex::new(BTreeMap::new());
static VERSION: AtomicU64 = AtomicU64::new(1);
static FRAMES: AtomicU64 = AtomicU64::new(0);
static STARTED: OnceLock<()> = OnceLock::new();
static CAMERA: Mutex<Option<Camera>> = Mutex::new(None);
static WINDOW: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
struct Layer {
    surface: Option<u64>,
    shapes: Vec<Value>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Camera {
    surface: u64,
    x: f64,
    y: f64,
    zoom: f64,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// where the local player looks: GameView's cached surface view (this peer's own camera)
fn camera() -> Option<Camera> {
    let scenario = fp::scenario()?;
    let view = "game._Mypair._Myval2.gameView.cachedSurfaceView";
    let get = |path: &str, depth: u32| -> Option<Value> {
        serde_json::from_str(&fp::read(scenario, "Scenario", &format!("{view}{path}"), depth).ok()?).ok()
    };
    let pos = get(".surfaceViewData.position.base", 1)?;
    Some(Camera {
        // (the engine counts surfaces from 0, Lua from 1)
        surface: get(".surfaceIndex", 0)?.as_u64()? + 1,
        x: pos["x"].as_f64()?,
        y: pos["y"].as_f64()?,
        zoom: get(".surfaceViewData.zoomData.zoom._Value", 0)?.as_f64()?,
    })
}

/// the game's main window: the biggest visible top-level window of this process
fn game_window() -> HWND {
    struct Best {
        hwnd: HWND,
        area: i64,
        mine: HWND,
    }
    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
        let best = &mut *(lp as *mut Best);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == GetCurrentProcessId() && hwnd != best.mine && IsWindowVisible(hwnd) != 0
            && GetWindow(hwnd, GW_OWNER).is_null() {
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut r);
            let area = (r.right - r.left) as i64 * (r.bottom - r.top) as i64;
            if area > best.area {
                best.area = area;
                best.hwnd = hwnd;
            }
        }
        1
    }
    let mut best = Best { hwnd: std::ptr::null_mut(), area: 0, mine: WINDOW.load(Relaxed) as HWND };
    unsafe { EnumWindows(Some(each), &mut best as *mut Best as LPARAM) };
    best.hwnd
}

fn color(v: &Value) -> Color {
    let c = |i: usize, d: f64| v.get(i).and_then(Value::as_f64).unwrap_or(d).clamp(0.0, 1.0) as f32;
    Color::from_rgba(c(0, 1.0), c(1, 1.0), c(2, 1.0), c(3, 1.0)).unwrap_or(Color::WHITE)
}

fn font() -> Option<&'static fontdue::Font> {
    static FONT: OnceLock<Option<fontdue::Font>> = OnceLock::new();
    FONT.get_or_init(|| {
        let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        ["segoeui.ttf", "arial.ttf"].iter()
            .find_map(|f| std::fs::read(format!(r"{dir}\Fonts\{f}")).ok())
            .and_then(|b| fontdue::Font::from_bytes(b, fontdue::FontSettings::default()).ok())
    }).as_ref()
}

/// text with its baseline-left at (x, y), alpha-blended into the pixmap
fn text(pm: &mut Pixmap, s: &str, x: f32, y: f32, size: f32, c: Color) {
    let Some(f) = font() else { return };
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    let mut pen = x;
    for ch in s.chars() {
        let (m, bitmap) = f.rasterize(ch, size);
        let ox = pen as i32 + m.xmin;
        let oy = y as i32 - m.height as i32 - m.ymin;
        for row in 0..m.height as i32 {
            for col in 0..m.width as i32 {
                let (px, py) = (ox + col, oy + row);
                if px < 0 || py < 0 || px >= w || py >= h {
                    continue;
                }
                let a = bitmap[(row * m.width as i32 + col) as usize] as f32 / 255.0 * c.alpha();
                if a <= 0.0 {
                    continue;
                }
                let i = ((py * w + px) * 4) as usize;
                // (premultiplied RGBA: source over)
                for (k, v) in [c.red(), c.green(), c.blue()].iter().enumerate() {
                    data[i + k] = (v * a * 255.0 + data[i + k] as f32 * (1.0 - a)).min(255.0) as u8;
                }
                data[i + 3] = (a * 255.0 + data[i + 3] as f32 * (1.0 - a)).min(255.0) as u8;
            }
        }
        pen += m.advance_width;
    }
}

fn render(pm: &mut Pixmap, cam: Camera) -> usize {
    let (cx, cy) = (pm.width() as f64 / 2.0, pm.height() as f64 / 2.0);
    let ppt = 32.0 * cam.zoom; // (pixels a tile at this zoom)
    let at = |p: &Value| -> Option<(f32, f32)> {
        let (x, y) = (p.get(0)?.as_f64()?, p.get(1)?.as_f64()?);
        Some(((cx + (x - cam.x) * ppt) as f32, (cy + (y - cam.y) * ppt) as f32))
    };
    let layers = LAYERS.lock().unwrap().clone();
    let mut n = 0;
    for layer in layers.values().filter(|l| l.surface.is_none_or(|s| s == cam.surface)) {
        for s in &layer.shapes {
            let mut paint = Paint::default();
            paint.set_color(color(&s["color"]));
            paint.anti_alias = true;
            let width = s["width"].as_f64().unwrap_or(2.0) as f32;
            let stroke = Stroke { width: width.max(0.5), ..Stroke::default() };
            let path = if let Some(l) = s.get("line") {
                let (Some(a), Some(b)) = (at(&l[0]), at(&l[1])) else { continue };
                let mut pb = PathBuilder::new();
                pb.move_to(a.0, a.1);
                pb.line_to(b.0, b.1);
                pb.finish().map(|p| (p, false))
            } else if let Some(r) = s.get("rect") {
                let (Some(a), Some(b)) = (at(&r[0]), at(&r[1])) else { continue };
                Rect::from_ltrb(a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
                    .map(|r| (PathBuilder::from_rect(r), width == 0.0))
            } else if let Some(c) = s.get("circle") {
                let Some(p) = at(c) else { continue };
                let r = (s["radius"].as_f64().unwrap_or(1.0) * ppt) as f32;
                PathBuilder::from_circle(p.0, p.1, r).map(|c| (c, width == 0.0))
            } else if let Some(t) = s.get("text").and_then(Value::as_str) {
                if let Some(p) = at(&s["at"]) {
                    text(pm, t, p.0, p.1, s["size"].as_f64().unwrap_or(16.0) as f32, color(&s["color"]));
                    n += 1;
                }
                continue;
            } else {
                continue;
            };
            if let Some((p, fill)) = path {
                if fill {
                    pm.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
                } else {
                    pm.stroke_path(&p, &paint, &stroke, Transform::identity(), None);
                }
                n += 1;
            }
        }
    }
    n
}

/// the pixmap (premultiplied RGBA) onto the layered window, over `game`'s client area
unsafe fn show(win: HWND, pm: &Pixmap, at: POINT) {
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let screen = GetDC(std::ptr::null_mut());
    let dc = CreateCompatibleDC(screen);
    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader = BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h,
        biPlanes: 1, biBitCount: 32, ..std::mem::zeroed() };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
    if !bmp.is_null() && !bits.is_null() {
        // (RGBA -> BGRA, both premultiplied)
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
        for (d, s) in dst.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = s[3];
        }
        let old = SelectObject(dc, bmp);
        let size = SIZE { cx: w, cy: h };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255,
                                    AlphaFormat: AC_SRC_ALPHA as u8 };
        UpdateLayeredWindow(win, screen, &at, &size, dc, &src, 0, &blend, ULW_ALPHA);
        SelectObject(dc, old);
        DeleteObject(bmp);
    }
    DeleteDC(dc);
    ReleaseDC(std::ptr::null_mut(), screen);
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, w: usize, l: isize) -> isize {
    if m == WM_NCHITTEST {
        return HTTRANSPARENT as isize;
    }
    DefWindowProcW(h, m, w, l)
}

/// the drawing thread: its own window and message loop; redraws when the camera, the layers or the size change
fn run() {
    unsafe {
        let class = wide("fse-draw");
        let inst = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
        RegisterClassW(&wc);
        let mut win: HWND = std::ptr::null_mut();
        let mut owner: HWND = std::ptr::null_mut();
        let mut last: Option<(Camera, u64, i32, i32, POINT)> = None;
        let mut shown = false;
        loop {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
            let game = game_window();
            let empty = LAYERS.lock().unwrap().is_empty();
            let cam = camera();
            *CAMERA.lock().unwrap() = cam;
            let visible = !game.is_null() && IsIconic(game) == 0 && !empty && cam.is_some();
            if game != owner && !game.is_null() {
                if !win.is_null() {
                    DestroyWindow(win);
                }
                // (owned by the game window: above it, below what covers it)
                win = CreateWindowExW(WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                                      class.as_ptr(), class.as_ptr(), WS_POPUP, 0, 0, 1, 1, game, std::ptr::null_mut(),
                                      inst, std::ptr::null());
                WINDOW.store(win as usize, Relaxed);
                owner = game;
                last = None;
                shown = false;
            }
            if win.is_null() {
                continue;
            }
            if !visible {
                if shown {
                    ShowWindow(win, SW_HIDE);
                    shown = false;
                }
                continue;
            }
            let cam = cam.unwrap();
            let mut r: RECT = std::mem::zeroed();
            GetClientRect(game, &mut r);
            let mut at = POINT { x: 0, y: 0 };
            ClientToScreen(game, &mut at);
            let (w, h) = (r.right.max(1), r.bottom.max(1));
            let key = (cam, VERSION.load(Relaxed), w, h, at);
            if last.is_some_and(|l| l.0 == key.0 && l.1 == key.1 && l.2 == key.2 && l.3 == key.3
                && l.4.x == key.4.x && l.4.y == key.4.y) {
                continue;
            }
            last = Some(key);
            let Some(mut pm) = Pixmap::new(w as u32, h as u32) else { continue };
            render(&mut pm, cam);
            show(win, &pm, at);
            FRAMES.fetch_add(1, Relaxed);
            if !shown {
                ShowWindow(win, SW_SHOWNOACTIVATE);
                shown = true;
            }
        }
    }
}

fn start() {
    STARTED.get_or_init(|| {
        std::thread::spawn(run);
    });
}

fn parse(text: &str) -> Result<Value, String> {
    serde_json::from_str(if text.trim().is_empty() { "{}" } else { text }).map_err(|e| format!("bad JSON: {e}"))
}

fp::export!(f_set, |_, text| {
    let v = parse(text)?;
    let id = v["id"].as_str().ok_or("no id")?.to_string();
    let shapes = v["shapes"].as_array().cloned().ok_or("no shapes")?;
    LAYERS.lock().unwrap().insert(id, Layer { surface: v["surface"].as_u64(), shapes });
    VERSION.fetch_add(1, Relaxed);
    start();
    Ok("ok".into())
});
fp::export!(f_clear, |_, text| {
    let v = parse(text)?;
    let mut l = LAYERS.lock().unwrap();
    match v["id"].as_str() {
        Some(id) => {
            l.remove(id);
        }
        None => l.clear(),
    }
    VERSION.fetch_add(1, Relaxed);
    Ok("ok".into())
});
fp::export!(f_status, |_, _| {
    let l = LAYERS.lock().unwrap();
    let cam = *CAMERA.lock().unwrap();
    Ok(json!({"layers": l.len(), "shapes": l.values().map(|x| x.shapes.len()).sum::<usize>(),
              "frames": FRAMES.load(Relaxed),
              "camera": cam.map(|c| json!({"surface": c.surface, "x": c.x, "y": c.y, "zoom": c.zoom}))}).to_string())
});

#[no_mangle]
pub unsafe extern "C" fn fse_plugin_init(host: *const fp::Host) -> i32 {
    if !fp::init(host, "draw") {
        return 1;
    }
    if fp::core_version() < (0, 10, 0) {
        fp::log("needs core 0.10.0: off");
        return 1;
    }
    for (f, n) in [(f_set as fp::PluginFn, "set"), (f_clear, "clear"), (f_status, "status")] {
        fp::register("draw", n, f, fp::THREADSAFE);
    }
    0
}
