"""the hub: the overlay button in the main menu (shown, at the window corner), hidden while a game ticks; the
in-game hub window's tabs (Hub, Mods, Startup, Get mods: screenshots), an install from a local mod index (hub-demo,
FSE_MOD_INDEX) and a page in the panel over the game (screenshot). Real client, vanilla + fse-std + fse-hub + fse-bridge."""
import ctypes, hashlib, io, json, os, shutil, subprocess, time, zipfile
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
# the catalog: a local index with one mod, hub-demo
demo = io.BytesIO()
with zipfile.ZipFile(demo, "w") as z:
    z.writestr("hub-demo/info.json", json.dumps({"name": "hub-demo", "version": "0.1.0", "title": "Hub demo",
        "author": "mtopfox", "factorio_version": "2.0", "dependencies": ["base"]}))
(RUN / "hub-demo_0.1.0.zip").write_bytes(demo.getvalue())
(RUN / "mod-index.json").write_text(json.dumps({"mods": [{"name": "hub-demo", "title": "Hub demo", "author": "mtopfox",
    "description": "installed by run_hub.py", "version": "0.1.0", "url": (RUN / "hub-demo_0.1.0.zip").as_uri(),
    "sha256": hashlib.sha256(demo.getvalue()).hexdigest(), "fse": "0.1.0", "plugins": ["py"]},
    {"name": "hub-needs", "title": "Needs a newer fse", "version": "1.0.0", "url": "file:///nowhere.zip", "sha256": "",
     "fse": "99.0.0", "plugins": ["nope"], "dependencies": ["base", "flib"]}]}))
os.environ["FSE_MOD_INDEX"] = (RUN / "mod-index.json").as_uri()
names = ["base", "elevated-rails", "quality", "space-age", "fse-std", "fse-hub", "fse-bridge", "hub-test"]
(MODS / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": True} for n in names]}))
launch = [str(NATIVE / "dist" / "fse-launcher.exe"), "--config", str(RUN / "config.ini"), "--mod-directory", str(MODS)]
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
installed = MODS / "hub-demo_0.1.0" / "info.json"
listed = {m["name"]: m["enabled"] for m in json.loads((MODS / "mod-list.json").read_text())["mods"]}
print("hub-demo installed:", installed.exists(), "enabled:", listed.get("hub-demo"))
print("screenshots:", sorted(f.name for f in OUT.glob("hub-*.png")))
