"""the draw plugin in a real client: the test mod draws around a chest and the player; this grabs the screen over the
game window (test/run/script-output/draw-screen.png: the drawing is a window of its own, not in game screenshots)
and prints draw.status. Don't run it while you play: it needs the game window in front."""
import ctypes, json, shutil, subprocess, sys, time
from ctypes import wintypes
from pathlib import Path

from PIL import ImageGrab

NATIVE = Path(__file__).resolve().parents[1]
from factorio_paths import run_dir  # noqa: E402
RUN = run_dir(Path(__file__).resolve().parent / "run")
MODS = RUN / "draw-mods"
OUT = RUN / "script-output"
shutil.rmtree(MODS, ignore_errors=True)
MODS.mkdir(parents=True)
shutil.copytree(NATIVE / "test" / "draw-test", MODS / "draw-test")
names = ["base", "draw-test"]
(MODS / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": n in names} for n in
                                                         names + ["elevated-rails", "quality", "space-age"]]}))
(OUT / "draw-status.txt").unlink(missing_ok=True)
save = RUN / "draw.zip"
save.unlink(missing_ok=True)
launch = [str(NATIVE / "dist" / "fse-launcher.exe"), "--config", str(RUN / "config.ini"), "--mod-directory", str(MODS)]
subprocess.run(launch + ["--create", str(save)], capture_output=True)
game = subprocess.Popen(launch + ["--load-game", str(save)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
user32 = ctypes.windll.user32
user32.SetProcessDPIAware()
try:
    end = time.time() + 180
    while time.time() < end and not (OUT / "draw-status.txt").exists():
        time.sleep(0.5)
    hwnd = user32.FindWindowW(None, None)
    found = []
    EnumProc = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def cb(h, _):
        buf = ctypes.create_unicode_buffer(256)
        user32.GetWindowTextW(h, buf, 256)
        if user32.IsWindowVisible(h) and buf.value.startswith("Factorio"):
            found.append(h)
        return True
    user32.EnumWindows(EnumProc(cb), 0)
    if found:
        user32.SetForegroundWindow(found[0])
        time.sleep(1.0)
        r = wintypes.RECT()
        user32.GetWindowRect(found[0], ctypes.byref(r))
        ImageGrab.grab(bbox=(r.left, r.top, r.right, r.bottom), all_screens=True).save(OUT / "draw-screen.png")
    status = (OUT / "draw-status.txt").read_text() if (OUT / "draw-status.txt").exists() else "no status"
    print("status:", status)
    print("screen:", OUT / "draw-screen.png" if found else "no game window")
    s = json.loads(status) if status.startswith("{") else {}
    sys.exit(0 if s.get("frames", 0) > 0 and s.get("camera") else 1)
finally:
    game.kill()
