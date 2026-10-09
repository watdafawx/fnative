"""fse-std in a real client (vanilla + fse-std + the demo + a test driver): drag & drop and resizing with
a mocked mouse; screenshots in test/run/script-output/fstd-*.png. The game window closes itself."""
import json, shutil, subprocess, time
from pathlib import Path

NATIVE = Path(__file__).resolve().parents[1]
from factorio_paths import run_dir  # noqa: E402
RUN = run_dir(Path(__file__).resolve().parent / "run")
MODS = RUN / "native-mods"
OUT = RUN / "script-output"
shutil.rmtree(MODS, ignore_errors=True)
MODS.mkdir(parents=True)
for m in ("fse-std", "fse-std-demo"):
    shutil.copytree(NATIVE / "mods" / m, MODS / m)
shutil.copytree(NATIVE / "test" / "std-test", MODS / "std-test")
(MODS / "std-test" / "info.json").write_text(json.dumps({"name": "std-test", "version": "0.0.1", "title": "std test",
    "author": "mtopfox", "factorio_version": "2.0", "dependencies": ["base", "fse-std-demo"]}))
names = ["base", "elevated-rails", "quality", "space-age", "fse-std", "fse-std-demo", "std-test"]
(MODS / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": True} for n in names]}))
for f in OUT.glob("fstd-*"):
    f.unlink()
save = RUN / "fstd.zip"
save.unlink(missing_ok=True)
launch = [str(NATIVE / "dist" / "fse.exe"), "--config", str(RUN / "config.ini"), "--mod-directory", str(MODS)]
subprocess.run(launch + ["--create", str(save)], capture_output=True)
game = subprocess.Popen(launch + ["--load-game", str(save)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
start = time.time()
while time.time() - start < 240 and not (OUT / "fstd-done.txt").exists() and game.poll() is None:
    time.sleep(1)
time.sleep(2)
game.kill()  # (the launcher: its job takes the game with it)
print((OUT / "fstd-result.txt").read_text() if (OUT / "fstd-result.txt").exists() else "no result")
print("screenshots:", sorted(f.name for f in OUT.glob("fstd-*.png")))
log = (RUN / "factorio-current.log").read_text(errors="replace").splitlines()
print("errors:", "\n".join(l for l in log if "Error" in l or "non-recoverable" in l)[-1500:] or "none")
