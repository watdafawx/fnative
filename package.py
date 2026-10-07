"""Packs a built dist/ into fnative-<version>-windows.zip (run python build.py first).

    python package.py

The zip holds the launcher, core, plugins, dashboard, the Python tools, the mods and install.ps1.
"""
import re
import zipfile
from pathlib import Path

NATIVE = Path(__file__).resolve().parent
DIST = NATIVE / "dist"
version = re.search(r'^version = "(.+)"', (NATIVE / "crates/fnative-core/Cargo.toml").read_text(), re.M).group(1)
out = NATIVE / f"fnative-{version}-windows.zip"
SKIP_DIST = {"fnative.env", "fnative.log", "mods-seen.json", "portal-cache.json", "web-token.txt", "cache", "webview"}
SKIP_MODS = {"bpgen-native-test"}

with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for f in sorted(DIST.rglob("*")):
        if f.is_file() and f.relative_to(DIST).parts[0] not in SKIP_DIST:
            z.write(f, f.relative_to(DIST))
    for f in sorted((NATIVE / "py").rglob("*")):
        if f.is_file() and "__pycache__" not in f.parts:
            z.write(f, f.relative_to(NATIVE))
    for f in sorted((NATIVE / "mods").rglob("*")):
        if f.is_file() and f.relative_to(NATIVE / "mods").parts[0] not in SKIP_MODS:
            z.write(f, f.relative_to(NATIVE))
    for name in ("install.ps1", "fnative.env.example", "README.md", "LICENSE"):
        if (NATIVE / name).exists():
            z.write(NATIVE / name, name)
print(out, f"{out.stat().st_size / 1e6:.1f} MB")
