"""headless: the core's engine API (native.read, layout, metatable, events) through dist/fse-launcher.exe, with the
engine-api-test mod; prints its report"""
import json, shutil, subprocess, sys
from pathlib import Path

from factorio_paths import run_dir

NATIVE = Path(__file__).resolve().parents[1]
RUN = run_dir(Path(__file__).resolve().parent / "run")
MODS = RUN / "engine-api-mods"
OUT = RUN / "script-output" / "engine-api.txt"

shutil.rmtree(MODS, ignore_errors=True)
MODS.mkdir(parents=True)
shutil.copytree(NATIVE / "test" / "engine-api-test", MODS / "engine-api-test")
shutil.copytree(NATIVE / "mods" / "fse-std", MODS / "fse-std")
names = ["base", "fse-std", "engine-api-test"]
(MODS / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": n in names} for n in
                                                         names + ["elevated-rails", "quality", "space-age"]]}))
OUT.unlink(missing_ok=True)
save = RUN / "engine-api.zip"
save.unlink(missing_ok=True)
launch = [str(NATIVE / "dist" / "fse-launcher.exe"), "--config", str(RUN / "config.ini"), "--mod-directory", str(MODS)]
for args in (["--create", str(save)], ["--benchmark", str(save), "--benchmark-ticks", "10", "--disable-audio"]):
    p = subprocess.run(launch + args, capture_output=True, text=True, encoding="utf-8", errors="replace")
    if p.returncode:
        print(" ".join(args[:1]), "exit", p.returncode, p.stdout[-1500:])
text = OUT.read_text() if OUT.exists() else "no report"
print(text)
sys.exit(0 if "ALL PASS" in text else 1)
