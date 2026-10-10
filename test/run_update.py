"""fse_tools.update without the game: a fake game folder (bin/x64/factorio.exe, fse/) and a fake release
(file:// API JSON, zip, .sha256) in test/run/update. check: newer or not, installable or not; install: files swapped
in, fse.env kept, a loaded DLL (LoadLibrary, as the running game holds fse.dll) renamed away and cleaned up by the
next check; a wrong sha256 changes nothing."""
import ctypes
import hashlib
import importlib
import io
import json
import os
import shutil
import sys
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
RUN = HERE / "run" / "update"
shutil.rmtree(RUN, ignore_errors=True)
GAME = RUN / "game"
HOME = GAME / "fse"
(GAME / "bin" / "x64").mkdir(parents=True)
(GAME / "bin" / "x64" / "factorio.exe").write_bytes(b"")
(HOME / "plugins").mkdir(parents=True)
(HOME / "fse.env").write_text("FSE_PYPATH=mine\n")
(HOME / "README.md").write_text("old")
# (a real DLL, loaded: Windows won't let anyone overwrite it, only rename it)
held = HOME / "plugins" / "held.dll"
shutil.copy2(Path(os.environ["SystemRoot"]) / "System32" / "version.dll", held)
k32 = ctypes.windll.kernel32
k32.LoadLibraryW.restype = ctypes.c_void_p
k32.FreeLibrary.argtypes = [ctypes.c_void_p]
handle = k32.LoadLibraryW(str(held))
os.environ.update(FSE_HOME=str(HOME), FSE_UPDATE_API=(RUN / "release.json").as_uri())
sys.path.insert(0, str(HERE.parent / "py"))
from fse_tools import update  # noqa: E402

fails = []


def check(what, ok):
    print(("ok   " if ok else "FAIL ") + what)
    if not ok:
        fails.append(what)


def release(version, sha=None):
    b = io.BytesIO()
    with zipfile.ZipFile(b, "w") as z:
        z.writestr("bin/x64/version.dll", "new loader " + version)
        z.writestr("fse/README.md", "new " + version)
        z.writestr("fse/fse.env", "FSE_PYPATH=example\n")
        z.writestr("fse/plugins/held.dll", "new plugin " + version)
        z.writestr("fse/plugins/added.dll", "added " + version)
    zf = RUN / f"fse-{version}-windows.zip"
    zf.write_bytes(b.getvalue())
    (RUN / (zf.name + ".sha256")).write_text((sha or hashlib.sha256(b.getvalue()).hexdigest()) + "  " + zf.name)
    (RUN / "release.json").write_text(json.dumps({"tag_name": "v" + version, "body": "notes " + version, "assets": [
        {"name": zf.name, "browser_download_url": zf.as_uri()},
        {"name": zf.name + ".sha256", "browser_download_url": (RUN / (zf.name + ".sha256")).as_uri()}]}))
    (HOME / "update-cache.json").unlink(missing_ok=True)


release("0.13.0")
r = json.loads(update.check("0.12.0"))
check("check: 0.13.0 newer than 0.12.0, installable " + json.dumps({k: r.get(k) for k in ("latest", "newer", "installable", "why")}),
      r["latest"] == "0.13.0" and r["newer"] and r["installable"])
check("check: not newer than itself", not json.loads(update.check("0.13.0"))["newer"])

release("0.13.0", sha="0" * 64)
r = json.loads(update.install())
check("wrong sha256: nothing written", "sha256" in r.get("error", "") and (HOME / "README.md").read_text() == "old")

release("0.13.0")
r = json.loads(update.install())
check("install: " + json.dumps(r), r.get("ok") and r["version"] == "0.13.0")
check("files swapped in", (HOME / "README.md").read_text() == "new 0.13.0"
      and (GAME / "bin" / "x64" / "version.dll").read_text() == "new loader 0.13.0"
      and (HOME / "plugins" / "added.dll").exists())
check("fse.env kept", (HOME / "fse.env").read_text() == "FSE_PYPATH=mine\n")
check("loaded DLL renamed away, new one in place", (HOME / "plugins" / "held.dll").read_text() == "new plugin 0.13.0"
      and (HOME / "plugins" / "held.dll.fse-old").exists())
check("staging folder gone", not (HOME / "update").exists())
k32.FreeLibrary(handle)
update.check("0.13.0")
check("next check deletes the .fse-old", not (HOME / "plugins" / "held.dll.fse-old").exists())

shutil.rmtree(GAME / "bin")
r = json.loads(update.check("0.12.0"))
check("dev build (no game folder): not installable", not r["installable"] and "game folder" in r["why"])
check("dev build: install refused", "game folder" in json.loads(update.install()).get("error", ""))

os.environ["FSE_UPDATE_API"] = (RUN / "missing.json").as_uri()
update = importlib.reload(update)
(HOME / "update-cache.json").unlink(missing_ok=True)
check("no network: an error, no crash", "error" in json.loads(update.check("0.12.0")))
print("PASS" if not fails else f"FAIL ({len(fails)})")
sys.exit(1 if fails else 0)
