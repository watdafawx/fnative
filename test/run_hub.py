"""the hub: the overlay button in the main menu (shown, at the window corner), hidden while a game ticks; the
in-game hub window's tabs (Hub, Mods, Startup: screenshots) and a page in the panel over the game (screenshot). Real client, vanilla + fse-std + fse-hub + fse-bridge."""
import ctypes, json, shutil, subprocess, time
from ctypes import wintypes
from pathlib import Path
from PIL import ImageGrab

NATIVE = Path(__file__).resolve().parents[1]
from factorio_paths import run_dir  # noqa: E402
RUN = run_dir(Path(__file__).resolve().parent / "run")
MODS = RUN / "native-mods"
OUT = RUN / "script-output"
u32 = ctypes.windll.user32
u32.SetProcessDPIAware()


def overlay():
    h = u32.FindWindowW("fse_overlay", None)
    if not h:
        return None
    r = wintypes.RECT()
    u32.GetWindowRect(h, ctypes.byref(r))
    return {"visible": bool(u32.IsWindowVisible(h)), "rect": (r.left, r.top, r.right, r.bottom)}


shutil.rmtree(MODS, ignore_errors=True)
MODS.mkdir(parents=True)
for m in ("fse-std", "fse-hub", "fse-bridge"):
    shutil.copytree(NATIVE / "mods" / m, MODS / m)
shutil.copytree(NATIVE / "test" / "hub-test", MODS / "hub-test")
(MODS / "hub-test" / "info.json").write_text(json.dumps({"name": "hub-test", "version": "0.0.1", "title": "hub test",
    "author": "mtopfox", "factorio_version": "2.0", "dependencies": ["base", "fse-hub"]}))
names = ["base", "elevated-rails", "quality", "space-age", "fse-std", "fse-hub", "fse-bridge", "hub-test"]
(MODS / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": True} for n in names]}))
launch = [str(NATIVE / "dist" / "fse.exe"), "--config", str(RUN / "config.ini"), "--mod-directory", str(MODS)]
save = RUN / "hub.zip"
save.unlink(missing_ok=True)
subprocess.run(launch + ["--create", str(save)], capture_output=True)
for f in OUT.glob("hub-*"):
    f.unlink()

# 1. the main menu
game = subprocess.Popen(launch, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
o = None
for _ in range(90):
    time.sleep(1)
    o = overlay()
    if o and o["visible"]:
        break
print("main menu overlay:", o)
time.sleep(3)
if o:
    l, t, r, b = o["rect"]
    ImageGrab.grab(bbox=(l - 12, t - 12, l + 700, t + 250), all_screens=True).save(OUT / "hub-menu.png")
game.kill()
time.sleep(3)

# 2. in a game
game = subprocess.Popen(launch + ["--load-game", str(save)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
seen = []
panel = None
start = time.time()
while time.time() - start < 300 and not (OUT / "hub-done.txt").exists() and game.poll() is None:
    time.sleep(1)
    o = overlay()
    if o:
        seen.append(o["visible"])
    h = u32.FindWindowW("fse_panel", None)
    if h and u32.IsWindowVisible(h) and panel is None:
        time.sleep(4)  # (the page loads)
        r = wintypes.RECT()
        u32.GetWindowRect(h, ctypes.byref(r))
        panel = (r.left, r.top, r.right, r.bottom)
        ImageGrab.grab(bbox=panel, all_screens=True).save(OUT / "hub-panel.png")
game.kill()
print("panel window:", panel)
print("overlay visible while the game ran (last 5 checks):", seen[-5:])
print((OUT / "hub-result.txt").read_text() if (OUT / "hub-result.txt").exists() else "no result")
print("screenshots:", sorted(f.name for f in OUT.glob("hub-*.png")))
