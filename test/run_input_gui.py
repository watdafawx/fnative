"""the input plugin in a real client: presses E through the game window's messages (WM_KEYDOWN/UP) when the test mod
asks; the first press must arrive as an action event, the second (that kind blocked) must not open the inventory.
Don't run it while you play: it needs the game window."""
import ctypes, json, shutil, subprocess, sys, time
from ctypes import wintypes
from pathlib import Path

NATIVE = Path(__file__).resolve().parents[1]
from factorio_paths import run_dir  # noqa: E402
RUN = run_dir(Path(__file__).resolve().parent / "run")
MODS = RUN / "input-mods"
OUT = RUN / "script-output"
shutil.rmtree(MODS, ignore_errors=True)
MODS.mkdir(parents=True)
shutil.copytree(NATIVE / "mods" / "fse-std", MODS / "fse-std")
shutil.copytree(NATIVE / "test" / "input-test", MODS / "input-test")
names = ["base", "fse-std", "input-test"]
(MODS / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": n in names} for n in
                                                         names + ["elevated-rails", "quality", "space-age"]]}))
for f in OUT.glob("input-*"):
    f.unlink()
save = RUN / "input.zip"
save.unlink(missing_ok=True)
launch = [str(NATIVE / "dist" / "fse-launcher.exe"), "--config", str(RUN / "config.ini"), "--mod-directory", str(MODS)]
subprocess.run(launch + ["--create", str(save)], capture_output=True)
game = subprocess.Popen(launch + ["--load-game", str(save)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

user32 = ctypes.windll.user32
EnumProc = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)


def game_windows():
    found = []
    def cb(hwnd, _):
        pid = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        buf = ctypes.create_unicode_buffer(256)
        user32.GetWindowTextW(hwnd, buf, 256)
        if user32.IsWindowVisible(hwnd) and buf.value.startswith("Factorio"):
            found.append(hwnd)
        return True
    user32.EnumWindows(EnumProc(cb), 0)
    return found


def press(vk):
    scan = user32.MapVirtualKeyW(vk, 0)
    for hwnd in game_windows():
        user32.SetForegroundWindow(hwnd)
        user32.PostMessageW(hwnd, 0x0100, vk, 1 | (scan << 16))                       # WM_KEYDOWN
        time.sleep(0.15)
        user32.PostMessageW(hwnd, 0x0101, vk, 1 | (scan << 16) | (3 << 30))           # WM_KEYUP


pressed = set()
start = time.time()
while time.time() - start < 240 and not (OUT / "input-done.txt").exists() and game.poll() is None:
    for n in (1, 2):
        if n not in pressed and (OUT / f"input-press-{n}.txt").exists():
            time.sleep(0.5)
            press(0x45)  # E
            pressed.add(n)
    time.sleep(0.2)
time.sleep(1)
game.kill()
text = (OUT / "input-result.txt").read_text() if (OUT / "input-result.txt").exists() else "no result"
print(text)
log = (RUN / "factorio-current.log").read_text(errors="replace").splitlines()
print("errors:", "\n".join(l for l in log if "Error" in l or "non-recoverable" in l)[-1500:] or "none")
sys.exit(0 if "FAIL" not in text and "PASS" in text else 1)
