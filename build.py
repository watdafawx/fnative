"""Builds fse into fse/dist: the launcher, the core DLL, the plugins (C and Python) and fse.env.

    python fse/build.py

Then start the game with dist/fse.exe [factorio arguments], or as a Steam launch option:
    "<this folder>\\dist\\fse.exe" %COMMAND%
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

NATIVE = Path(__file__).resolve().parent
REL = NATIVE / "target" / "release"
DIST = NATIVE / "dist"
PYTHON = sys.executable
MODS = ("fse-std", "fse-hub", "fse-bridge", "fse-agent")  # (not the demos and test mods)


def run(cmd, cwd=None, env=None):
    p = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True)
    if p.returncode:
        print(p.stdout[-4000:], p.stderr[-4000:])
        sys.exit(1)


env = dict(os.environ, PYO3_PYTHON=PYTHON)  # (the Python plugin is built against this interpreter)
run(["cargo", "build", "--release"], cwd=NATIVE, env=env)
run([PYTHON, str(NATIVE / "test" / "build_plugins.py")])
(DIST / "plugins").mkdir(parents=True, exist_ok=True)
for name in ("fse.exe", "fse.dll"):
    shutil.copy2(REL / name, DIST / name)
(DIST / "bin" / "x64").mkdir(parents=True, exist_ok=True)
shutil.copy2(REL / "version.dll", DIST / "bin" / "x64" / "version.dll")
for dll in (REL / "plugins").glob("*.dll"):
    shutil.copy2(dll, DIST / "plugins" / dll.name)
for cfg in (NATIVE / "plugins-config").glob("*.json"):  # (a plugin's settings: kept once the user edits them)
    dest = DIST / "plugins" / cfg.name
    if cfg.name.endswith(".needs.json") or not dest.exists():
        shutil.copy2(cfg, dest)
for needs in (NATIVE / "needs").glob("*.needs.json"):  # (what our code relies on in the engine: checked each build)
    shutil.copy2(needs, DIST / needs.name)
shutil.copytree(NATIVE / "web", DIST / "web", dirs_exist_ok=True)  # (the dashboard pages)
shutil.copytree(NATIVE / "py", DIST / "py", dirs_exist_ok=True, ignore=shutil.ignore_patterns("__pycache__"))
for mod in MODS:  # (the loader puts these into the game's mods folder)
    shutil.rmtree(DIST / "mods" / mod, ignore_errors=True)
    shutil.copytree(NATIVE / "mods" / mod, DIST / "mods" / mod)
shutil.copy2(NATIVE / "uninstall.cmd", DIST / "uninstall.cmd")
if not (DIST / "fse.env").exists():  # (the user's own settings are kept)
    shutil.copy2(NATIVE / "fse.env.example", DIST / "fse.env")
print("built", DIST, sorted(p.name for p in DIST.rglob("*") if p.is_file()))
