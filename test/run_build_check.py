"""the build check: first start of a build writes a report, the next one knows it; a fake earlier build in the
cache shows what moved. Runs headless vanilla through dist/ (needs the game closed? no: it is a separate process)"""
import json, shutil, subprocess, os, time
from pathlib import Path

NATIVE = Path(__file__).resolve().parents[1]
DIST = NATIVE / "dist"
from factorio_paths import run_dir  # noqa: E402
RUN = run_dir(Path(__file__).resolve().parent / "run")
MODS = RUN / "native-mods"
LOG = DIST / "fse.log"


def start():
    LOG.unlink(missing_ok=True)
    save = RUN / "native-demo.zip"
    if not save.exists():
        subprocess.run([str(DIST / "fse.exe"), "--config", str(RUN / "config.ini"), "--mod-directory",
                        str(MODS), "--create", str(save)], capture_output=True)
    p = subprocess.run([str(DIST / "fse.exe"), "--config", str(RUN / "config.ini"), "--mod-directory",
                        str(MODS), "--benchmark", str(save), "--benchmark-ticks", "5", "--disable-audio"],
                       capture_output=True, text=True, errors="replace")
    return p.returncode, [l.split(" ", 1)[1] for l in LOG.read_text().splitlines() if "build" in l or "  " in l or "missing" in l]


shutil.rmtree(DIST / "cache", ignore_errors=True)
print("first start:", *start(), sep="\n  ")
print("second start:", *start(), sep="\n  ")
# a game update, faked: an "older build" whose Inserter fields sit elsewhere, and this build's cache gone
cache = DIST / "cache"
real = next(cache.iterdir())
fake = cache / "00000000000000000000000000000000-1"
fake.mkdir()
e = json.loads((real / "engine.json").read_text())
e["build"] = {"guid": "00000000000000000000000000000000", "age": 1}
for f in e["classes"]["Inserter"]["fields"]:
    if f["name"] in ("heldStack", "dropTarget"):
        f["offset"] -= 8
e["classes"]["Inserter"]["size"] -= 8
(fake / "engine.json").write_text(json.dumps(e))
os.utime(fake / "engine.json", (time.time() - 3600, time.time() - 3600))
shutil.rmtree(real)
print("after a (fake) update:", *start(), sep="\n  ")
print("report file:")
print(next(p for p in cache.iterdir() if p != fake).joinpath("report.txt").read_text())
