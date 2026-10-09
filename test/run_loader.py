"""headless: the installed loader (python install.py first). factorio.exe started directly, the way Steam starts it:
fse loads by itself (native-demo sees it) and puts the bundled mods into the mods folder. With FSE_OFF the game runs
plain; through the launcher too, fse loads once per start (two starts each: create, run)."""
import json, os, shutil, subprocess
from pathlib import Path

from factorio_paths import PATHS, run_dir

NATIVE = Path(__file__).resolve().parents[1]
RUN = run_dir(Path(__file__).resolve().parent / "run")
MODS = RUN / "loader-mods"
EXE = PATHS["factorio_exe"]
HOME = EXE.parents[2] / "fse"
LOG = HOME / "fse.log"
GAME_LOG = RUN / "factorio-current.log"
assert (EXE.parent / "version.dll").exists() and (HOME / "fse.dll").exists(), "not installed: python install.py"


def start(exe, env):
    shutil.rmtree(MODS, ignore_errors=True)
    MODS.mkdir(parents=True)
    shutil.copytree(NATIVE / "mods" / "native-demo", MODS / "native-demo")
    names = ["base", "elevated-rails", "quality", "space-age", "native-demo"]
    (MODS / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": True} for n in names]}))
    LOG.unlink(missing_ok=True)
    save = RUN / "loader.zip"
    save.unlink(missing_ok=True)
    common = ["--config", str(RUN / "config.ini"), "--mod-directory", str(MODS)]
    for args in (["--create", str(save)], ["--benchmark", str(save), "--benchmark-ticks", "60", "--disable-audio"]):
        subprocess.run([str(exe)] + common + args, capture_output=True, env=env)
    log = LOG.read_text(errors="replace") if LOG.exists() else ""
    game = GAME_LOG.read_text(errors="replace")
    errors = [l for l in game.splitlines() if "Error" in l]
    return {"fse": "data stage sees fse" in game, "errors": errors[:3], "loads": log.count(" loading"),
            "mods": sorted(p.name for p in MODS.iterdir() if p.name.startswith("fse-"))}


on = {k: v for k, v in os.environ.items() if k != "FSE_OFF"}
on["FSE_PYPATH"] = str(NATIVE / "test" / "pymods")
direct = start(EXE, on)
off = start(EXE, dict(on, FSE_OFF="1"))
launched = start(NATIVE / "dist" / "fse.exe", dict(on, FSE_LOG=str(LOG)))
print("direct:", direct, "\nFSE_OFF:", off, "\nlauncher:", launched)
assert direct == {"fse": True, "errors": [], "loads": 2, "mods": ["fse-agent", "fse-bridge", "fse-hub", "fse-std"]}, direct
assert not off["fse"] and off["loads"] == 0, off
assert launched["fse"] and launched["loads"] == 2 and not launched["errors"], launched
print("ok")
