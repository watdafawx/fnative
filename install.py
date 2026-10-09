"""Installs this build (dist/) into the game for development: the loader into <game>/bin/x64 and <game>/fse as a
junction to dist/, so every rebuild is live at the next start. Users unzip the release instead.

    python install.py            install (the game found as the tests find it: FACTORIO_EXE, else Steam)
    python install.py --remove   uninstall (the junction only: dist/ is untouched)
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

NATIVE = Path(__file__).resolve().parent
sys.path.insert(0, str(NATIVE / "test"))
from factorio_paths import PATHS  # noqa: E402

exe = PATHS["factorio_exe"]
loader, home = exe.parent / "version.dll", exe.parents[2] / "fse"
if subprocess.run(["tasklist", "/fi", "imagename eq factorio.exe"], capture_output=True, text=True).stdout.count(
        "factorio.exe"):
    sys.exit("close Factorio first")
if "--remove" in sys.argv:
    loader.unlink(missing_ok=True)
    if home.is_junction():
        os.rmdir(home)
    print("removed from", exe.parents[2])
    sys.exit()
if home.exists() and not home.is_junction():
    sys.exit(f"{home} is a real install (from the release zip): run its uninstall.cmd first")
shutil.copy2(NATIVE / "dist" / "bin" / "x64" / "version.dll", loader)
if not home.exists():
    subprocess.run(["cmd", "/c", "mklink", "/J", str(home), str(NATIVE / "dist")], check=True, capture_output=True)
print(f"installed: {loader}\n           {home} -> {NATIVE / 'dist'}")
