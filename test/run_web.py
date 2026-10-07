"""the web API end to end: a headless vanilla game through dist/ with fnative-bridge + fnative-agent; HTTP from here"""
import json, shutil, subprocess, time, urllib.request, urllib.error
from pathlib import Path

NATIVE = Path(__file__).resolve().parents[1]
DIST = NATIVE / "dist"
from factorio_paths import run_dir  # noqa: E402
RUN = run_dir(Path(__file__).resolve().parent / "run")
MODS = RUN / "native-mods"
BASE = "http://127.0.0.1:8790"

shutil.rmtree(MODS, ignore_errors=True)
MODS.mkdir(parents=True)
for m in ("fnative-bridge", "fnative-agent"):
    shutil.copytree(NATIVE / "mods" / m, MODS / m)
names = ["base", "elevated-rails", "quality", "space-age", "fnative-bridge", "fnative-agent"]
(MODS / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": True} for n in names]}))
save = RUN / "native-web.zip"
save.unlink(missing_ok=True)
launch = [str(DIST / "factorio-native.exe"), "--config", str(RUN / "config.ini"), "--mod-directory", str(MODS)]
subprocess.run(launch + ["--create", str(save)], capture_output=True)
(DIST / "web-token.txt").unlink(missing_ok=True)
# a long benchmark: the game keeps ticking while we talk to it
game = subprocess.Popen(launch + ["--benchmark", str(save), "--benchmark-ticks", "1000000", "--disable-audio"],
                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def req(method, path, body=None, token=True):
    t = (DIST / "web-token.txt").read_text().strip()
    r = urllib.request.Request(BASE + path, method=method, data=json.dumps(body).encode() if body is not None else None,
                               headers={"Authorization": f"Bearer {t}"} if token else {})
    try:
        with urllib.request.urlopen(r, timeout=15) as f:
            return f.status, json.loads(f.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


def game_call(iface, fn, *args):
    return req("POST", "/api/game", {"interface": iface, "function": fn, "args": list(args)})


try:
    for _ in range(120):
        if (DIST / "web-token.txt").exists():
            try:
                if req("GET", "/api/status")[1].get("game_thread_answering"):
                    break
            except Exception:
                pass
        time.sleep(0.5)
    print("no token:", req("GET", "/api/status", token=False))
    print("status:", req("GET", "/api/status"))
    print("$game status:", game_call("$game", "status"))
    code, ifaces = game_call("$game", "interfaces")
    print("interfaces:", code, {k: len(v) for k, v in (ifaces.get("result") or {}).items()})
    print("spawn:", game_call("agent", "spawn", "bob", 0, 0))
    print("give:", game_call("agent", "give", "bob", "wooden-chest", 5))
    code, seen = game_call("agent", "look", "bob", 20)
    r = seen.get("result") or {}
    print("look:", code, len(r.get("entities", [])), "entities,", r.get("resources"))
    print("walk:", game_call("agent", "walk_to", "bob", 6, 3))
    time.sleep(3)
    print("events:", game_call("agent", "events", "bob"))
    print("place:", game_call("agent", "place", "bob", "wooden-chest", 7, 3))
    print("place far:", game_call("agent", "place", "bob", "wooden-chest", 70, 3))
    print("say:", game_call("agent", "say", "bob", "hello from outside"))
    print("status:", game_call("agent", "status", "bob"))
    print("unknown:", game_call("agent", "fly", "bob"))
    print("profiler:", req("POST", "/api/profiler/start"))
    time.sleep(2)
    code, prof = req("GET", "/api/profile?consumer=test")
    print("profile:", code, prof.get("ticks"), "ticks;", [(f["name"], round(f["ms_per_tick"], 3)) for f in prof.get("functions", [])[:4]])
    print("native py:", req("POST", "/api/native/hello/reverse", "abc"))
finally:
    game.kill()
print("--- fnative.log tail")
print("\n".join((DIST / "fnative.log").read_text().splitlines()[-6:]))
