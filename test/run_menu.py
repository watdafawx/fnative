"""the "FSE hub" button in the game's menus (web plugin, menu.rs), in a real client (vanilla + fse-std):
  1. the main menu: screenshot, the button clicked, the hub panel must open; the overlay buttons stay hidden
  2. a game, Esc: the pause menu, the same
Input is posted to the game's window and screenshots come from the window itself, so another window may be in front.
The clicks: python run_menu.py [main_x main_y [pause_x pause_y]] (client pixels, read off the screenshots in
test/run/menu/script-output); without them only the screenshots are taken."""
import ctypes, json, shutil, subprocess, sys, time
from ctypes import wintypes
from pathlib import Path

NATIVE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from factorio_paths import PATHS, run_dir  # noqa: E402
# (a fresh write-data folder: a game killed mid-write leaves a config.ini whose reset dialog covers the menu)
shutil.rmtree(Path(__file__).resolve().parent / "run" / "menu", ignore_errors=True)
RUN = run_dir(Path(__file__).resolve().parent / "run" / "menu")
MODS = RUN / "menu-mods"
OUT = RUN / "script-output"
OUT.mkdir(parents=True, exist_ok=True)
LOG = NATIVE / "dist" / "fse.log"
u32 = ctypes.windll.user32
u32.SetProcessDPIAware()
fails = []


def check(what, ok):
    print(("ok   " if ok else "FAIL ") + what)
    if not ok:
        fails.append(what)


def visible(cls):
    h = u32.FindWindowW(cls, None)
    return bool(h and u32.IsWindowVisible(h))


def game_window():
    out = subprocess.run(["tasklist", "/FI", "IMAGENAME eq factorio.exe", "/FO", "CSV", "/NH"], capture_output=True,
                         text=True).stdout
    pids = {int(l.split('","')[1]) for l in out.splitlines() if l.startswith('"factorio')}
    found = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def each(h, _):
        p = wintypes.DWORD()
        u32.GetWindowThreadProcessId(h, ctypes.byref(p))
        if p.value in pids and u32.IsWindowVisible(h) and u32.GetWindowTextLengthW(h) > 0:
            found.append(h)
        return True
    u32.EnumWindows(each, 0)
    return found[0] if found else None


def grab(h, path):
    """the game window's own picture (PrintWindow), whatever is in front of it"""
    from PIL import Image
    g32 = ctypes.windll.gdi32
    r = wintypes.RECT()
    u32.GetClientRect(h, ctypes.byref(r))
    w, ht = r.right, r.bottom
    dc = u32.GetDC(h)
    mem = g32.CreateCompatibleDC(dc)
    bmp = g32.CreateCompatibleBitmap(dc, w, ht)
    g32.SelectObject(mem, bmp)
    u32.PrintWindow(h, mem, 3)  # PW_CLIENTONLY | PW_RENDERFULLCONTENT
    buf = ctypes.create_string_buffer(w * ht * 4)
    g32.GetBitmapBits(bmp, len(buf), buf)
    Image.frombuffer("RGBA", (w, ht), buf, "raw", "BGRA", 0, 1).convert("RGB").save(path)
    g32.DeleteObject(bmp)
    g32.DeleteDC(mem)
    u32.ReleaseDC(h, dc)


def focus(h):
    # (the game takes posted input only while it thinks it has the focus)
    u32.PostMessageW(h, 0x0006, 1, 0)  # WM_ACTIVATE
    u32.PostMessageW(h, 0x0007, 0, 0)  # WM_SETFOCUS
    time.sleep(0.3)


def key(h, vk, scan):
    focus(h)
    u32.PostMessageW(h, 0x0100, vk, 1 | scan << 16)  # WM_KEYDOWN
    time.sleep(0.05)
    u32.PostMessageW(h, 0x0101, vk, 1 | scan << 16 | 3 << 30)  # WM_KEYUP


def click(h, x, y):
    focus(h)
    at = y << 16 | x
    u32.PostMessageW(h, 0x0200, 0, at)  # WM_MOUSEMOVE
    time.sleep(0.3)
    u32.PostMessageW(h, 0x0201, 1, at)  # WM_LBUTTONDOWN
    time.sleep(0.05)
    u32.PostMessageW(h, 0x0202, 0, at)  # WM_LBUTTONUP


def phase(name, args, at, settle):
    """start the game (args), settle(h) brings up the menu; screenshot; click at, the panel must open"""
    LOG.write_text("")
    game = subprocess.Popen([str(NATIVE / "dist" / "fse-launcher.exe"), "--config", str(RUN / "config.ini"),
                             "--mod-directory", str(MODS)] + args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        h = None
        for _ in range(120):
            time.sleep(1)
            h = game_window()
            if h and "Factorio initialised" in (RUN / "factorio-current.log").read_text(errors="replace"):
                break
        check(f"{name}: game window", h is not None)
        if not h:
            return
        time.sleep(8)
        settle(h)
        time.sleep(2)
        grab(h, OUT / f"menu-{name}.png")
        print(f"{name}: screenshot", OUT / f"menu-{name}.png")
        check(f"{name}: overlay buttons hidden", not visible("fse_overlay"))
        if at:
            # (a posted click now and then misses the game's input loop: up to three)
            for _ in range(3):
                click(h, *at)
                for _ in range(10):
                    time.sleep(0.5)
                    if visible("fse_panel"):
                        break
                if visible("fse_panel"):
                    break
            check(f"{name}: the click opens the panel", visible("fse_panel"))
    finally:
        game.kill()
        subprocess.run(["taskkill", "/F", "/IM", "factorio.exe"], capture_output=True)
        time.sleep(3)


shutil.rmtree(MODS, ignore_errors=True)
MODS.mkdir(parents=True)
shutil.copytree(NATIVE / "mods" / "fse-std", MODS / "fse-std")
# (no crash-site intro in the save: it asks for Tab and pauses the game)
(MODS / "menu-test").mkdir()
(MODS / "menu-test" / "info.json").write_text(json.dumps({"name": "menu-test", "version": "0.0.1", "title": "menu test",
    "author": "mtopfox", "factorio_version": "2.0", "dependencies": ["base"]}))
(MODS / "menu-test" / "control.lua").write_text("""script.on_init(function()
  local fp = remote.interfaces["freeplay"]
  if fp then
    if fp.set_skip_intro then remote.call("freeplay", "set_skip_intro", true) end
    if fp.set_disable_crashsite then remote.call("freeplay", "set_disable_crashsite", true) end
  end
end)
""")
(MODS / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": True} for n in ("base", "fse-std", "menu-test")]
    + [{"name": n, "enabled": False} for n in ("elevated-rails", "quality", "space-age")]}))
save = RUN / "menu.zip"
subprocess.run([str(PATHS["factorio_exe"]), "--config", str(RUN / "config.ini"), "--mod-directory", str(MODS),
                "--create", str(save)], capture_output=True)
xy = [int(a) for a in sys.argv[1:5]]

phase("main", [], xy[0:2], lambda h: None)
check("hooks installed in both menus (fse.log)",
      "FSE hub button under Settings in: main menu, pause menu" in LOG.read_text(errors="replace"))
phase("pause", ["--load-game", str(save)], xy[2:4], lambda h: key(h, 0x1B, 0x01))  # Esc
print("PASS" if not fails else f"FAIL ({len(fails)})")
sys.exit(1 if fails else 0)
