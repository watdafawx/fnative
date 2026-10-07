"""Builds fnative into native/dist: the launcher, the core DLL, the plugins (C and Python) and fnative.env.

    python native/build.py

Then start the game with dist/factorio-native.exe [factorio arguments], or as a Steam launch option:
    "<this folder>\\dist\\factorio-native.exe" %COMMAND%
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


def run(cmd, cwd=None, env=None):
    p = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True)
    if p.returncode:
        print(p.stdout[-4000:], p.stderr[-4000:])
        sys.exit(1)


env = dict(os.environ, PYO3_PYTHON=PYTHON)  # (the Python plugin is built against this interpreter)
run(["cargo", "build", "--release"], cwd=NATIVE, env=env)
run([PYTHON, str(NATIVE / "test" / "build_plugins.py")])
(DIST / "plugins").mkdir(parents=True, exist_ok=True)
for name in ("factorio-native.exe", "fnative.dll"):
    shutil.copy2(REL / name, DIST / name)
for dll in (REL / "plugins").glob("*.dll"):
    shutil.copy2(dll, DIST / "plugins" / dll.name)
for cfg in (NATIVE / "plugins-config").glob("*.json"):  # (a plugin's settings: kept once the user edits them)
    dest = DIST / "plugins" / cfg.name
    if cfg.name.endswith(".needs.json") or not dest.exists():
        shutil.copy2(cfg, dest)
for needs in (NATIVE / "needs").glob("*.needs.json"):  # (what our code relies on in the engine: checked each build)
    shutil.copy2(needs, DIST / needs.name)
shutil.copytree(NATIVE / "web", DIST / "web", dirs_exist_ok=True)  # (the dashboard pages)
if not (DIST / "fnative.env").exists():  # (the user's own settings are kept)
    text = (NATIVE / "fnative.env.example").read_text(encoding="utf-8")
    (DIST / "fnative.env").write_text(text.replace("{FNATIVE_PY}", str(NATIVE / "py")), encoding="utf-8")
print("built", DIST, sorted(p.name for p in DIST.rglob("*") if p.is_file()))
