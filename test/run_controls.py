"""input.controls: the game's controls and a mod's custom inputs, with their bindings (fse/dist). A real game window
(the controls exist only with graphics): it opens for a few seconds and closes itself."""
import ctypes, json, os, shutil, subprocess, time
from ctypes import wintypes
from pathlib import Path

NATIVE = Path(__file__).resolve().parents[1]
DIST = NATIVE / "dist"
from factorio_paths import run_dir  # noqa: E402
RUN = run_dir(Path(__file__).resolve().parent / "run")
MODS = RUN / "controls-mods"
u32 = ctypes.windll.user32


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


def focus(h):
    u32.PostMessageW(h, 0x0006, 1, 0)  # WM_ACTIVATE
    u32.PostMessageW(h, 0x0007, 0, 0)  # WM_SETFOCUS


shutil.rmtree(MODS, ignore_errors=True)
MODS.mkdir(parents=True)
shutil.copytree(Path(__file__).parent / "controls-test", MODS / "controls-test")
(MODS / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": True} for n in ("base", "controls-test")]}))
out = RUN / "script-output" / "controls-test.json"
out.unlink(missing_ok=True)
live = RUN / "script-output" / "controls-live.txt"
live.unlink(missing_ok=True)
save = RUN / "controls-test.zip"
save.unlink(missing_ok=True)
common = [str(DIST / "fse-launcher.exe"), "--config", str(RUN / "config.ini"), "--mod-directory", str(MODS)]
subprocess.run(common + ["--create", str(save)], capture_output=True)
env = dict(os.environ, SteamAppId="427520", SteamGameId="427520")
game = subprocess.Popen(common + ["--load-game", str(save.resolve()), "--disable-audio"], stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL, env=env)
start = time.time()
focused = 0
while time.time() - start < 120 and not live.exists() and game.poll() is None:
    h = game_window() if focused < 3 else None
    if h:
        focused += 1  # (a few times while it loads; focusing again mid-press would reset the held modifiers)
        focus(h)  # (posted keys count only while the game thinks it has the focus)
    time.sleep(0.5)
time.sleep(1)
game.kill()
text = out.read_text() if out.exists() else "no output"
try:
    controls = {c["name"]: c for c in json.loads(text)}
except ValueError:
    print(text[:2000]); raise SystemExit(1)
fails = 0


def check(name, ok):
    global fails
    fails += not ok
    print("PASS" if ok else "FAIL", name)


for n in ("toggle-map", "undo", "copy-entity-settings", "controls-test-combo", "controls-test-mouse", "toggle-driving"):
    print(n, json.dumps({k: v for k, v in controls.get(n, {}).items()}))
print(len(controls), "controls")
keys = lambda n: controls.get(n, {}).get("keys", [{}])
check("toggle-map is Tab, then M", [k.get("scancode") for k in keys("toggle-map")] == ["SDL_SCANCODE_TAB", "SDL_SCANCODE_M"])
check("undo is ctrl+Z", keys("undo")[0].get("mods") == ["ctrl"])
check("a mod's custom input is there, modded", controls.get("controls-test-combo", {}).get("modded") is True)
check("shift+alt+K", sorted(keys("controls-test-combo")[0].get("mods", [])) == ["alt", "shift"])
check("mouse button 4", keys("controls-test-mouse")[0].get("mouse") == "button-4")
check("copy settings: shift + right click", keys("copy-entity-settings")[0] == {"mouse": "button-2", "mods": ["shift"], "modifiers": 2})
events = live.read_text().splitlines() if live.exists() else ["no live result"]
print(*events, sep=chr(10))
check("trigger fires a mod's custom input", any(e.startswith("combo@12") for e in events))
check("press fires it too", any(e.startswith("combo@18") for e in events))
check("press: mouse button 4", any(e.startswith("mouse@24") for e in events))
check("send_action LuaShortcut raises on_lua_shortcut", any(e.startswith("controls-test-sc@") and e.endswith(":p1") for e in events))
check("trigger toggle-map opens the map", "render_mode 2" in events)
raise SystemExit(1 if fails else 0)
