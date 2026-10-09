"""Packs a built dist/ into fse-<version>-windows.zip (run python build.py first).

    python package.py

The zip mirrors the game's folder: extracted into it, bin/x64/version.dll lands beside factorio.exe and the rest in
<game>/fse. That is the whole install; fse/uninstall.cmd undoes it.
"""
import re
import zipfile
from pathlib import Path

NATIVE = Path(__file__).resolve().parent
DIST = NATIVE / "dist"
version = re.search(r'^version = "(.+)"', (NATIVE / "crates/fse-core/Cargo.toml").read_text(), re.M).group(1)
out = NATIVE / f"fse-{version}-windows.zip"
# (this machine's state, the launcher (the loader replaces it), the C example plugin)
SKIP = {"fse.env", "fse.log", "mods-seen.json", "portal-cache.json", "web-token.txt", "cache", "webview", "overlay.json", "fse.exe",
        "hello.dll", "__pycache__"}

with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for f in sorted(DIST.rglob("*")):
        rel = f.relative_to(DIST)
        if f.is_file() and not SKIP & set(rel.parts):
            z.write(f, rel if rel.parts[0] == "bin" else Path("fse") / rel)
    z.write(NATIVE / "fse.env.example", "fse/fse.env")
    for name in ("README.md", "LICENSE"):
        if (NATIVE / name).exists():
            z.write(NATIVE / name, f"fse/{name}")
print(out, f"{out.stat().st_size / 1e6:.1f} MB")
