"""Hot reload for mod development: the fse-hotreload mod calls these every half second in a singleplayer game.

FSE_HOTRELOAD (fse.env): ';'-separated folders, each a mod's source (has info.json) or a folder of them. A watched mod
the game runs from a folder in its mods folder (FSE_MODS) is scanned for changed files by a background thread (a scan
takes milliseconds: not on the game thread). `check` hands the changed .lua sources to Lua, which compiles them first
(a syntax error in a reloaded control.lua ends the game, one in data.lua stops the restart at an error screen);
`apply` copies the source over the installed folder. Control-stage code only: Lua calls game.reload_script. Anything
else (data*.lua, settings*.lua, prototypes/, info.json, locale, graphics): prototypes only load at the game's start
(game.reload_mods reloads scripts only, even in a client), so `restart` has the game saved and started again on that
save. Edits made while the game is closed aren't copied.

    check {"mods": [active mod names]} -> {"changed": [{"name", "lua": {path: source}, "restart": [paths]}], "notes": []}
    apply [names]                      -> "[]"
    restart {"save": name}             -> "" or an error (then Lua calls game.auto_save(name))
"""
import json
import os
import shutil
import subprocess
import threading
import time
from pathlib import Path

SKIP = {"test", ".git", "__pycache__", ".vscode"}
_lock = threading.Lock()
_sources = None   # name -> source folder
_watch = set()    # names the thread scans
_pending = {}     # name -> changed relative paths, not yet handed out
_noted = set()


def _find_sources():
    out = {}
    # (FSE_HOTRELOAD_TEST: run_hotreload.py's, which fse.env can't override)
    folders = os.environ.get("FSE_HOTRELOAD_TEST") or os.environ.get("FSE_HOTRELOAD", "")
    for entry in filter(None, (e.strip() for e in folders.split(";"))):
        p = Path(entry)
        dirs = [p] if (p / "info.json").exists() else [c for c in p.iterdir() if (c / "info.json").exists()] \
            if p.is_dir() else []
        for d in dirs:
            try:
                out.setdefault(json.loads((d / "info.json").read_text(encoding="utf-8"))["name"], d)
            except (OSError, ValueError, KeyError):
                pass
    return out


def _files(d):
    out = {}
    for root, dirs, files in os.walk(d):
        dirs[:] = [x for x in dirs if x not in SKIP]
        for f in files:
            p = os.path.join(root, f)
            try:
                out[os.path.relpath(p, d).replace("\\", "/")] = os.stat(p).st_mtime_ns
            except OSError:  # (an editor's temp file already gone)
                pass
    return out


def _scan():
    seen = {}
    while True:
        with _lock:
            names = list(_watch)
        for name in names:
            now = _files(_sources[name])
            before = seen.get(name)
            seen[name] = now
            if before is not None and now != before:
                paths = {p for p in now if before.get(p) != now[p]} | (before.keys() - now.keys())
                with _lock:
                    _pending.setdefault(name, set()).update(paths)
        time.sleep(0.5)


def _control_code(path):
    name = path.rsplit("/", 1)[-1]
    return name.endswith(".lua") and not name.startswith(("data", "settings")) and not path.startswith("prototypes/")


def check(s):
    global _sources
    installed = Path(os.environ.get("FSE_MODS", ""))
    notes = []
    if _sources is None:
        _sources = _find_sources()
        threading.Thread(target=_scan, daemon=True, name="fse-hotreload").start()
    with _lock:
        for name in json.loads(s)["mods"]:
            if name in _watch or name not in _sources:
                continue
            if (installed / name).is_dir():
                _watch.add(name)
            elif name not in _noted:
                _noted.add(name)
                notes.append(f"{name} isn't a folder in {installed}: hot reload needs it unzipped")
        pending = dict(_pending)
        _pending.clear()
    changed = []
    for name, paths in sorted(pending.items()):
        src = _sources[name]
        lua = {p: (src / p).read_text(encoding="utf-8", errors="replace") for p in sorted(paths)
               if p.endswith(".lua") and (src / p).exists()}
        changed.append({"name": name, "lua": lua, "restart": sorted(p for p in paths if not _control_code(p))})
    return json.dumps({"changed": changed, "notes": notes})


def apply(s):
    installed = Path(os.environ["FSE_MODS"])
    for name in json.loads(s):
        dest = installed / name
        shutil.rmtree(dest, ignore_errors=True)
        shutil.copytree(_sources[name], dest, ignore=shutil.ignore_patterns(*SKIP))
    return "[]"


# arguments that pick what the game does at start (dropped on a restart: it loads the save instead), and how many
# values each takes
GAME_ARGS = {"--load-game": 1, "--create": 1, "--start-server": 1, "--mp-connect": 1, "--benchmark": 1,
             "--benchmark-ticks": 1, "--load-scenario": 1, "--start-server-load-scenario": 1, "--map-gen-settings": 1,
             "--map-settings": 1}

# waits for the game (pid) to end, then starts the command (a separate process: the game holds its write-data lock)
WAIT_THEN_START = """import ctypes, subprocess, sys
h = ctypes.windll.kernel32.OpenProcess(0x00100000, False, int(sys.argv[1]))
if h:
    ctypes.windll.kernel32.WaitForSingleObject(h, 120000)
subprocess.Popen(sys.argv[2:])
"""


def _game_exe():
    import ctypes
    buf = ctypes.create_unicode_buffer(32768)
    ctypes.windll.kernel32.GetModuleFileNameW(None, buf, len(buf))
    return buf.value


def _arguments():
    """this game's command line arguments, without the program and the ones in GAME_ARGS"""
    import ctypes
    k32, shell = ctypes.windll.kernel32, ctypes.windll.shell32
    k32.GetCommandLineW.restype = ctypes.c_wchar_p
    shell.CommandLineToArgvW.restype = ctypes.POINTER(ctypes.c_wchar_p)
    n = ctypes.c_int()
    argv = shell.CommandLineToArgvW(k32.GetCommandLineW(), ctypes.byref(n))
    out, skip = [], 0
    for a in (argv[i] for i in range(1, n.value)):
        if skip:
            skip -= 1
        elif a in GAME_ARGS:
            skip = GAME_ARGS[a]
        else:
            out.append(a)
    return out


def restart(s):
    """{"save": name}: Lua calls game.auto_save(name) right after this. Once the log says the save is written, the game
    is started again on it (through fse's launcher when the loader isn't installed, with this game's arguments) and
    this process ends. Answers an error text, or "" when it's on its way."""
    write = Path(os.environ.get("FSE_WRITE_DATA", ""))
    log = write / "factorio-current.log"
    save = write / "saves" / ("_autosave-" + json.loads(s)["save"] + ".zip")
    python = shutil.which("python") or shutil.which("py")
    if not python:
        return "no python on PATH to start the game again"
    launcher = Path(os.environ.get("FSE_HOME", "")) / "fse-launcher.exe"
    exe = _game_exe() if os.environ.get("FSE_LOADER") == "1" or not launcher.exists() else str(launcher)
    seen = log.stat().st_size

    def wait_and_go():
        end = time.time() + 120
        while time.time() < end:
            with open(log, "rb") as f:
                f.seek(seen)
                if b"Saving finished" in f.read():
                    break
            time.sleep(0.1)
        else:
            print("fse-hotreload: the save didn't finish in 2 minutes, not restarting")
            return
        subprocess.Popen([python, "-c", WAIT_THEN_START, str(os.getpid()), exe, *_arguments(), "--load-game", str(save)],
                         creationflags=subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP,
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os._exit(0)

    threading.Thread(target=wait_and_go, daemon=True, name="fse-hotreload restart").start()
    return ""
