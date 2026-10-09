"""plugin "fixes", data-cache-tilde: a one-file mod with a "~" dependency, headless vanilla with cache-prototype-data on.
Plain game: "Data stage cache not used" at every start; through fse: "loaded" from the second start."""
import json, re, shutil, subprocess
from pathlib import Path

from factorio_paths import PATHS

NATIVE = Path(__file__).resolve().parents[1]
from factorio_paths import run_dir  # noqa: E402
RUN = run_dir(Path(__file__).resolve().parent / "run")
ROOT = RUN / "tilde-test"
EXE = PATHS["factorio_exe"]

shutil.rmtree(ROOT, ignore_errors=True)
mod = ROOT / "mods" / "tilde-probe"
mod.mkdir(parents=True)
(mod / "info.json").write_text(json.dumps({"name": "tilde-probe", "version": "0.0.1", "title": "t", "author": "t",
                                           "factorio_version": "2.0", "dependencies": ["base", "~ quality"]}))
(mod / "data.lua").write_text("-- probe\n")
(ROOT / "mods" / "mod-list.json").write_text(json.dumps({"mods": [{"name": "base", "enabled": True},
                                                                  {"name": "tilde-probe", "enabled": True}]}))
(ROOT / "data").mkdir()
cfg = re.sub(r"(?m)^write-data=.*$", "write-data=" + (ROOT / "data").as_posix(), (RUN / "config.ini").read_text())
(ROOT / "c.ini").write_text(cfg.replace("[other]", "[other]\ncache-prototype-data=true", 1) if "[other]" in cfg
                            else cfg + "\n[other]\ncache-prototype-data=true\n")


def start(exe):
    subprocess.run([str(exe), "--config", str(ROOT / "c.ini"), "--mod-directory", str(ROOT / "mods"), "--create",
                    str(ROOT / "t.zip")], capture_output=True)
    log = (ROOT / "data" / "factorio-current.log").read_text(errors="replace")
    return "loaded" if "Data stage cache loaded" in log else "not used"


plain = [start(EXE), start(EXE)]
fixed = [start(NATIVE / "dist" / "fse.exe"), start(NATIVE / "dist" / "fse.exe")]
print("plain game:", plain, " through fse:", fixed)
assert plain == ["not used", "not used"] and fixed == ["loaded", "loaded"], "unexpected"
print("ok")
