"""Where Factorio lives on this machine, for the test scripts (no other dependencies).

  factorio_exe   FACTORIO_EXE, else the first Steam library that has the game
  game_data      the game's data folder (next to bin/)
  write_data     FACTORIO_WRITE_DATA, else what the game's config-path.cfg / config.ini say, else %APPDATA%/Factorio
  user_mods      FACTORIO_MODS, else <write_data>/mods
  script_output  <write_data>/script-output

headless() runs one test case (a scenario mod) in a headless game; main() is a mod's whole test/run.py.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

os.environ.setdefault("FSE_OFF", "1")  # (games started from here run plain even with the fse loader installed)

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
    # (without the version line the game calls the file invalid and asks, over the main menu, to reset it)
    text = f"; version=13\n[path]\nread-data={PATHS['game_data'].as_posix()}\nwrite-data={path.as_posix()}\n"
    cur = ini.read_text(encoding="utf-8") if ini.exists() else ""
    if "; version=" not in cur or f"write-data={path.as_posix()}" not in cur:
        ini.write_text(text, encoding="utf-8")  # (written again when the folder has moved)
    return path


BASE_MODS = ["base", "elevated-rails", "quality", "space-age"]


def user_mod(name):
    """the newest <name>_*.zip in the game's own mods folder"""
    zips = PATHS["user_mods"].glob(name + "_*.zip")
    return max(zips, key=lambda z: [int(n) for n in re.findall(r"\d+", z.stem)])


def stage_mods(md, mods, base=BASE_MODS):
    """`mods` ({name: folder or zip}) copied into the mod folder `md` (emptied first), with a mod-list.json"""
    shutil.rmtree(md, ignore_errors=True)
    md.mkdir(parents=True)
    for name, src in mods.items():
        if src.is_dir():
            shutil.copytree(src, md / name, ignore=shutil.ignore_patterns("test", ".git", "__pycache__"))
        else:
            shutil.copy(src, md)
    (md / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": True} for n in [*base, *mods]]}))


def headless(run, mods, case, ticks, base=BASE_MODS):
    """Copy `mods` ({name: folder or zip}) into <run>/headless-mods, create a map and run it `ticks` ticks headless.
    Returns (ok, text): ok when the game logged no error and script-output/<case>.txt exists without a FAIL line."""
    run = run_dir(run)
    md = run / "headless-mods"
    stage_mods(md, mods, base)
    out = run / "script-output" / (case + ".txt")
    out.unlink(missing_ok=True)
    save = run / "headless.zip"
    save.unlink(missing_ok=True)
    common = [str(PATHS["factorio_exe"]), "--config", str(run / "config.ini"), "--mod-directory", str(md)]
    for args in (["--create", str(save)], ["--benchmark", str(save), "--benchmark-ticks", str(ticks), "--disable-audio"]):
        # (below normal priority: test games only get the CPU the desktop isn't using)
        p = subprocess.run(common + args, capture_output=True, text=True, encoding="utf-8", errors="replace",
                           creationflags=subprocess.BELOW_NORMAL_PRIORITY_CLASS)
        if p.returncode or "non-recoverable" in p.stdout or "Error" in p.stdout:
            return False, p.stdout[-3000:]
    text = out.read_text(encoding="utf-8", errors="replace") if out.exists() else "no result"
    return out.exists() and "FAIL" not in text, text


def main(run_py, cases, base=BASE_MODS, user_mods=()):
    """A mod's test/run.py. `cases` is {case: ticks}, the first one the default.
      run.py [CASE [TICKS]] [--with MOD]   one case in test/run (MOD: another mod from the game's own mods folder)
      run.py --all [-j N]                  every case, N games at once (default 4), each in test/run/all/<case>
    Exits 1 when a case fails."""
    here = Path(run_py).resolve().parent
    mod = here.parent
    ap = argparse.ArgumentParser(usage=main.__doc__.splitlines()[1].strip() + "\n       "
                                 + main.__doc__.splitlines()[2].strip())
    ap.add_argument("case", nargs="?", default=next(iter(cases)))
    ap.add_argument("ticks", nargs="?", type=int)
    ap.add_argument("--with", dest="extra", action="append", default=[])
    ap.add_argument("--all", action="store_true")
    ap.add_argument("-j", type=int, default=4)
    a = ap.parse_args()
    extra = {n: user_mod(n) for n in [*user_mods, *a.extra]}

    def one(case, ticks, run):
        return headless(run, {mod.name: mod, **extra, case: here / case}, case, ticks, base)

    if not a.all:
        ok, text = one(a.case, a.ticks or cases.get(a.case) or next(iter(cases.values())), here / "run")
        print(text)
        sys.exit(0 if ok else 1)
    start, failed = time.time(), []
    with ThreadPoolExecutor(a.j) as pool:
        # longest first, so a slow one doesn't start last
        jobs = {pool.submit(one, c, t, here / "run" / "all" / c): c for c, t in sorted(cases.items(), key=lambda ct: -ct[1])}
        for f in as_completed(jobs):
            ok, text = f.result()
            print(f"{'PASS' if ok else 'FAIL'} {jobs[f]} ({time.time() - start:.0f} s)", flush=True)
            if not ok:
                failed.append(jobs[f])
                print("    " + "\n    ".join(text.strip().splitlines()[-15:]), flush=True)
    print(f"{len(cases) - len(failed)}/{len(cases)} passed in {time.time() - start:.0f} s"
          + (": failed " + " ".join(sorted(failed)) if failed else ""))
    sys.exit(1 if failed else 0)
