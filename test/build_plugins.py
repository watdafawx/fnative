"""builds the example plugins into target/release/plugins (the launcher's plugins folder)"""
import shutil, subprocess, sys
from pathlib import Path

NATIVE = Path(__file__).resolve().parents[1]
OUT = NATIVE / "target" / "release" / "plugins"
OUT.mkdir(parents=True, exist_ok=True)
CLANG = shutil.which("clang-cl") or r"C:\Program Files\LLVM\bin\clang-cl.exe"

def run(cmd, cwd=None):
    p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if p.returncode:
        print(p.stdout[-3000:], p.stderr[-3000:])
        sys.exit(1)

# C (the example plugin: skipped without clang-cl, nothing else needs it)
if Path(CLANG).exists():
    run([CLANG, "/nologo", "/LD", "/O2", "/I", str(NATIVE / "include"), "hello.c", "/Fe:" + str(OUT / "hello.dll")],
        cwd=NATIVE / "plugins" / "hello-c")
else:
    print("clang-cl not found: the C example plugin (hello.dll) is skipped")
for junk in OUT.glob("hello.*"):
    if junk.suffix in (".lib", ".exp", ".obj"):
        junk.unlink()
# Python (built by cargo with the workspace)
# Rust plugins (built by cargo with the workspace)
for name in ("fnative_python.dll", "fnative_profiler.dll", "fnative_web.dll", "fnative_std.dll", "fnative_diag.dll", "fnative_fixes.dll"):
    dll = NATIVE / "target" / "release" / name
    if dll.exists():
        shutil.copy(dll, OUT / name)
print("plugins:", sorted(p.name for p in OUT.glob("*.dll")))
