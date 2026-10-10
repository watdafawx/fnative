"""Where Factorio lives on this machine, for the test scripts (no other dependencies).

  factorio_exe   FACTORIO_EXE, else the first Steam library that has the game
  game_data      the game's data folder (next to bin/)
  write_data     FACTORIO_WRITE_DATA, else what the game's config-path.cfg / config.ini say, else %APPDATA%/Factorio
  user_mods      FACTORIO_MODS, else <write_data>/mods
  script_output  <write_data>/script-output
"""
import os
import re
from pathlib import Path

os.environ.setdefault("FSE_OFF", "1")  # (games started from here run plain even with the fse loader installed)
os.environ.setdefault("FSE_NO_STEAM", "1")  # (and without Steam: they never touch the player's Steam settings)

STEAM_GUESSES = [Path(p) / "steamapps/common/Factorio" for p in (
    "C:/Program Files (x86)/Steam", "C:/Program Files/Steam", "D:/SteamLibrary", "E:/SteamLibrary", "F:/SteamLibrary",
    "G:/SteamLibrary")]


def _game_root():
    if os.environ.get("FACTORIO_EXE"):
        return Path(os.environ["FACTORIO_EXE"]).parents[2]
    for g in STEAM_GUESSES:
        if (g / "bin/x64/factorio.exe").exists():
            return g
    return STEAM_GUESSES[0]


def _write_data(root):
    if os.environ.get("FACTORIO_WRITE_DATA"):
        return Path(os.environ["FACTORIO_WRITE_DATA"])
    appdata = Path(os.environ.get("APPDATA", Path.home() / "AppData/Roaming")) / "Factorio"
    try:
        cfgpath = (root / "config-path.cfg").read_text(encoding="utf-8")
        m = re.search(r"^config-path=(.*)$", cfgpath, re.M)
        config_dir = m.group(1).strip() if m else "__PATH__system-write-data__/config"
        config_dir = config_dir.replace("__PATH__system-write-data__", str(appdata)).replace(
            "__PATH__executable__", str(root / "bin/x64"))
        ini = (Path(config_dir) / "config.ini").read_text(encoding="utf-8")
        m = re.search(r"^write-data=(.*)$", ini, re.M)
        if m:
            return Path(m.group(1).strip().replace("__PATH__system-write-data__", str(appdata))
                        .replace("__PATH__executable__", str(root / "bin/x64")))
    except OSError:
        pass
    return appdata


_root = _game_root()
_write = _write_data(_root)
PATHS = {"factorio_exe": Path(os.environ.get("FACTORIO_EXE") or _root / "bin/x64/factorio.exe"),
         "game_data": _root / "data", "write_data": _write,
         "user_mods": Path(os.environ.get("FACTORIO_MODS") or _write / "mods"),
         "script_output": _write / "script-output"}


def run_dir(path):
    """a write-data folder for headless runs, with its own config.ini, so tests never touch the game's own"""
    path.mkdir(parents=True, exist_ok=True)
    ini = path / "config.ini"
    text = f"[path]\nread-data={PATHS['game_data'].as_posix()}\nwrite-data={path.as_posix()}\n"
    if not ini.exists() or f"write-data={path.as_posix()}" not in ini.read_text(encoding="utf-8"):
        ini.write_text(text, encoding="utf-8")  # (written again when the folder has moved)
    return path
