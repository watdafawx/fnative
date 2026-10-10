"""fse-hotreload in a real game window (through dist/fse-launcher.exe): a probe mod's source is edited while the game
runs. control.lua: copied in and reloaded, storage kept; a syntax error refused, the game lives on. data.lua: the
change is held (a button waits) until fse-hotreload's remote restart (here over fse's web API): then the game is saved
and started again on that save, with the new prototype, the new control.lua and the storage. Opens game windows: don't
run it while you play. Needs fse built (python build.py) with fse/py on FSE_PYPATH, and python on PATH."""
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

from factorio_paths import run_dir

NATIVE = Path(__file__).resolve().parents[1]
RUN = run_dir(Path(__file__).resolve().parent / "run" / "hotreload")
MODS = RUN / "mods"
SRC = RUN / "src" / "hr-probe"
OUT = RUN / "script-output" / "hr-probe.txt"
CONTROL = """VERSION = {v}
script.on_init(function()
  storage.n = 7
  if remote.interfaces.freeplay then
    remote.call("freeplay", "set_disable_crashsite", true)
    remote.call("freeplay", "set_skip_intro", true)
  end
end)
script.on_nth_tick(30, function()
  helpers.write_file("hr-probe.txt", "v" .. VERSION .. " storage " .. storage.n .. " stack " ..
    prototypes.item["iron-plate"].stack_size .. ";", true)
end)
"""
DATA = 'data.raw.item["iron-plate"].stack_size = {n}\n'

for d in (MODS, SRC.parent, RUN / "saves"):
    shutil.rmtree(d, ignore_errors=True)
SRC.mkdir(parents=True)
(SRC / "info.json").write_text(json.dumps({"name": "hr-probe", "version": "0.1.0", "title": "hr probe", "author": "fse",
                                           "factorio_version": "2.0", "dependencies": ["base"]}))
(SRC / "control.lua").write_text(CONTROL.format(v=1))
(SRC / "data.lua").write_text(DATA.format(n=111))
shutil.copytree(SRC, MODS / "hr-probe")
for f in (MODS / "hr-probe").iterdir():  # (installed as an unzip does it: same bytes, new file times)
    os.utime(f, (time.time() + 60, time.time() + 60))
for m in ("fse-hotreload", "fse-bridge"):
    shutil.copytree(NATIVE / "mods" / m, MODS / m)
names = ["base", "fse-hotreload", "fse-bridge", "hr-probe"]
(MODS / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": n in names} for n in
                                                         names + ["elevated-rails", "quality", "space-age"]]}))
save = RUN / "saves" / "hotreload.zip"
save.parent.mkdir(parents=True)
env = {k: v for k, v in os.environ.items() if k != "FSE_OFF"} | {"FSE_HOTRELOAD_TEST": str(SRC.parent)}
launch = [str(NATIVE / "dist" / "fse-launcher.exe"), "--config", str(RUN / "config.ini"), "--mod-directory", str(MODS)]
p = subprocess.run(launch + ["--create", str(save)], capture_output=True, text=True, encoding="utf-8", errors="replace",
                   env=env)
if not save.exists():
    sys.exit("the map wasn't created:\n" + p.stdout[-2000:])
OUT.unlink(missing_ok=True)
subprocess.Popen(launch + ["--load-game", str(save), "--disable-audio"], env=env,
                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
failed = []


def games():
    """pids of this test's games (the first one and the one a restart starts)"""
    ps = ("Get-CimInstance Win32_Process -Filter \"Name='factorio.exe'\" | Where-Object { $_.CommandLine -like '*"
          + str(RUN) + "*' } | ForEach-Object { $_.ProcessId }")
    return subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True).stdout.split()


def log():
    text = ""
    for f in ("factorio-previous.log", "factorio-current.log"):
        if (RUN / f).exists():
            text += (RUN / f).read_text(encoding="utf-8", errors="replace")
    return text


def last():
    lines = OUT.read_text().split(";") if OUT.exists() else []
    return lines[-2] if len(lines) > 1 else ""


def wait(want, secs=90):
    end = time.time() + secs
    while time.time() < end and last() != want:
        time.sleep(0.25)
    return last() == want


def web_restart():
    """what an agent does when its edits are done: remote.call("fse-hotreload", "restart") through fse-bridge"""
    token = (NATIVE / "dist" / "web-token.txt").read_text().strip()
    port = next((l.split("=", 1)[1].strip() for l in (NATIVE / "dist" / "fse.env").read_text().splitlines()
                 if l.startswith("FSE_WEB_PORT=")), "8790")
    body = json.dumps({"interface": "fse-hotreload", "function": "restart", "args": []}).encode()
    r = urllib.request.Request(f"http://127.0.0.1:{port}/api/game", data=body, method="POST",
                               headers={"Authorization": f"Bearer {token}"})
    try:
        with urllib.request.urlopen(r, timeout=15) as f:
            return f.status, json.loads(f.read()).get("result")
    except (urllib.error.URLError, OSError) as e:
        return 0, str(e)


def check(name, ok, got=""):
    print(("PASS " if ok else "FAIL ") + name + (" :: " + got if got else ""))
    if not ok:
        failed.append(name)


try:
    check("game running with the probe", wait("v1 storage 7 stack 111"), last())
    time.sleep(1.5)
    check("an installed copy with other file times but the same bytes isn't a change",
          "hr-probe held" not in log() and "hr-probe reloaded" not in log(), "")
    (SRC / "README.md").write_text("notes\n")
    time.sleep(2)
    check("a README edit: copied, no hold, no reload",
          (MODS / "hr-probe" / "README.md").exists() and "hr-probe held" not in log()
          and "hr-probe reloaded" not in log(), "")
    pid = games()
    time.sleep(1.5)  # (the first look only notes the files)
    (SRC / "control.lua").write_text(CONTROL.format(v=2))
    check("control.lua edit reloaded, storage kept", wait("v2 storage 7 stack 111", 15), last())
    check("installed copy updated", "VERSION = 2" in (MODS / "hr-probe" / "control.lua").read_text())
    (SRC / "control.lua").write_text("VERSION = = 3\n")
    time.sleep(3)
    check("syntax error refused, old code kept", "hr-probe not reloaded" in log() and last() == "v2 storage 7 stack 111"
          and "VERSION = 2" in (MODS / "hr-probe" / "control.lua").read_text(), last())
    (SRC / "control.lua").write_text(CONTROL.format(v=4))
    check("fixed file reloaded", wait("v4 storage 7 stack 111", 15), last())
    (SRC / "data.lua").write_text(DATA.format(n=222))
    time.sleep(2)
    (SRC / "control.lua").write_text(CONTROL.format(v=5))  # (a held mod's control.lua waits too)
    time.sleep(3)
    check("data.lua edit held: no restart, nothing copied, control.lua waits too",
          "hr-probe held" in log() and last() == "v4 storage 7 stack 111" and games() == pid
          and "111" in (MODS / "hr-probe" / "data.lua").read_text(), last())
    t0 = time.time()
    status, answer = web_restart()
    check("restart through the web API (remote fse-hotreload.restart)", status == 200 and "restarting" in str(answer),
          f"{status} {answer}")
    check("restarted on the save: new prototype and control.lua, storage kept", wait("v5 storage 7 stack 222", 120),
          f"{last()} after {time.time() - t0:.0f} s")
    check("a new game process", games() and games() != pid, f"{pid} -> {games()}")
    check("the restart loaded the hot reload save", "_autosave-hotreload" in log())
finally:
    for g in games():
        subprocess.run(["taskkill", "/F", "/PID", g], capture_output=True)
errors = [l for l in log().splitlines() if "Error" in l]
check("no errors in the logs", not errors, "\n".join(errors[-5:]))
print("ALL PASS" if not failed else "FAILED: " + ", ".join(failed))
sys.exit(1 if failed else 0)
