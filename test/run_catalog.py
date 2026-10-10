"""fse_tools.catalog without the game: a file:// index and mods folder in test/run/catalog; listing, install, an
update replacing the older copy, FSE_PYPATH in fse.env, and the refusals (wrong sha256, a working copy installed)."""
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
RUN = HERE / "run" / "catalog"
shutil.rmtree(RUN, ignore_errors=True)
(RUN / "mods").mkdir(parents=True)
(RUN / "home").mkdir()
(RUN / "home" / "fse.env").write_text("FSE_PYPATH=C:\\other\n")
os.environ.update(FSE_MODS=str(RUN / "mods"), FSE_HOME=str(RUN / "home"),
                  FSE_MOD_INDEX=(RUN / "index.json").as_uri())
sys.path.insert(0, str(HERE.parent / "py"))
from fse_tools import catalog  # noqa: E402

catalog = importlib.reload(catalog)


def mod_zip(version, extra=None):
    b = io.BytesIO()
    with zipfile.ZipFile(b, "w") as z:
        z.writestr("demo/info.json", json.dumps({"name": "demo", "version": version, "dependencies": ["base", "flib"]}))
        z.writestr("demo/control.lua", "-- demo " + version)
        for n, s in (extra or {}).items():
            z.writestr(n, s)
    f = RUN / f"demo_{version}.zip"
    f.write_bytes(b.getvalue())
    return f


def publish(version, sha=None, extra=None):
    f = mod_zip(version, extra)
    (RUN / "index.json").write_text(json.dumps({"mods": [{
        "name": "demo", "title": "Demo", "version": version, "url": f.as_uri(), "pypath": True,
        "sha256": sha or hashlib.sha256(f.read_bytes()).hexdigest(), "dependencies": ["base", "flib"]}]}))


fails = []


def check(what, ok):
    print(("ok   " if ok else "FAIL ") + what)
    if not ok:
        fails.append(what)


mods = RUN / "mods"
publish("1.0.0")
r = json.loads(catalog.listing())
check("listing: not installed, flib missing", r["mods"][0]["installed"] is None and r["mods"][0]["missing"] == ["flib"])
r = json.loads(catalog.install("demo"))
check("install 1.0.0: " + json.dumps(r), r.get("ok") and (mods / "demo_1.0.0" / "control.lua").exists())
check("enabled in mod-list.json", {"name": "demo", "enabled": True} in json.loads((mods / "mod-list.json").read_text())["mods"])
env = (RUN / "home" / "fse.env").read_text()
check("on FSE_PYPATH", env.strip() == "FSE_PYPATH=C:\\other;" + str(mods / "demo_1.0.0"))
check("listing: installed 1.0.0", json.loads(catalog.listing())["mods"][0]["installed"] == "1.0.0")

publish("1.1.0")
r = json.loads(catalog.install("demo"))
check("update to 1.1.0 replaces 1.0.0", r.get("ok") and r["replaced"] == ["demo_1.0.0"]
      and not (mods / "demo_1.0.0").exists() and (mods / "demo_1.1.0").exists())
env = (RUN / "home" / "fse.env").read_text()
check("FSE_PYPATH has 1.1.0 only", "demo_1.0.0" not in env and "demo_1.1.0" in env)

publish("1.2.0", sha="0" * 64)
r = json.loads(catalog.install("demo"))
check("wrong sha256 refused", "sha256" in r.get("error", "") and (mods / "demo_1.1.0").exists())

publish("1.2.0", extra={"other/x.lua": ""})
check("two top folders refused", "one folder" in json.loads(catalog.install("demo")).get("error", ""))

publish("1.2.0")
(mods / "demo").mkdir()
(mods / "demo" / "info.json").write_text(json.dumps({"name": "demo", "version": "9.0.0"}))
(mods / "demo" / ".git").mkdir()
r = json.loads(catalog.install("demo"))
check("working copy left alone", "working copy" in r.get("error", "") and (mods / "demo" / ".git").exists())

check("unknown mod", "not in the index" in json.loads(catalog.install("nope")).get("error", ""))
os.environ["FSE_MOD_INDEX"] = (RUN / "missing.json").as_uri()
catalog = importlib.reload(catalog)
r = json.loads(catalog.listing())
check("no index: the cached one, with the error", r.get("error") and r["mods"][0]["name"] == "demo")

print("PASS" if not fails else f"FAIL ({len(fails)})")
sys.exit(1 if fails else 0)
