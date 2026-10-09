"""headless proof: vanilla + native-demo, started through fse-launcher.exe; prints the demo's report and log"""
import json, shutil, subprocess, sys
from pathlib import Path

NATIVE = Path(__file__).resolve().parents[1]
ROOT = NATIVE.parent / "bpgen"  # (bpgen, beside fse in the dev repo)
from factorio_paths import run_dir  # noqa: E402
RUN = run_dir(Path(__file__).resolve().parent / "run")
MODS = RUN / "native-mods"
LAUNCH = NATIVE / "target" / "release" / "fse-launcher.exe"
LOG = NATIVE / "target" / "release" / "fse.log"
OUT = RUN / "script-output" / "native-demo.txt"

shutil.rmtree(MODS, ignore_errors=True)
MODS.mkdir(parents=True)
shutil.copytree(NATIVE / "mods" / "native-demo", MODS / "native-demo")
names = ["base", "elevated-rails", "quality", "space-age", "native-demo"]
(MODS / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": True} for n in names]}))
LOG.unlink(missing_ok=True)
OUT.unlink(missing_ok=True)
save = RUN / "native-demo.zip"
save.unlink(missing_ok=True)
import os
os.environ["FSE_PYPATH"] = str(NATIVE / "test" / "pymods")
common = ["--config", str(RUN / "config.ini"), "--mod-directory", str(MODS)]
for args in (["--create", str(save)], ["--benchmark", str(save), "--benchmark-ticks", "180", "--disable-audio"]):
    p = subprocess.run([str(LAUNCH)] + common + args, capture_output=True, text=True, encoding="utf-8", errors="replace")
    print(" ".join(args[:1]), "exit", p.returncode, p.stderr.strip()[-500:])
print("--- report"); print(OUT.read_text() if OUT.exists() else "no report")
print("--- fse.log"); print(LOG.read_text() if LOG.exists() else "no log")
log = (RUN / "factorio-current.log").read_text(errors="replace").splitlines()
print("--- game log"); print("\n".join(l for l in log if "native-demo" in l or "Error" in l)[-1500:])
