"""multiplayer on this machine: a headless server and a game client, both through dist/fse-launcher.exe, with the
mp-test mod (its state changes only through fse's simulation events and native.sync). Passes when both peers see
the same rotations and synced clocks and neither log has a desync; then a plain client (FSE_OFF) must be kicked.
Opens game windows: don't run it while you play. Port 34297 on 127.0.0.1.

Other projects use it for their own multiplayer tests:
    run_mp.py --mod <mod folder>... --expect <regex> [--no-kick] [--first]
loads those mods (fse-std always) instead of mp-test; each peer of the test mod writes "tick N <state>" lines to
script-output/mp-server.txt (for_player 0) and mp-player-1.txt (for_player 1) every few seconds; the client's must
match the server's at the same ticks and its last one match --expect. --no-kick (or --first) skips the plain client."""
import argparse, json, os, re, shutil, subprocess, sys, time
from pathlib import Path

NATIVE = Path(__file__).resolve().parents[1]
ap = argparse.ArgumentParser()
ap.add_argument("--mod", action="append", type=Path)
ap.add_argument("--expect", default=r"synced [1-9]")
ap.add_argument("--no-kick", action="store_true")
ap.add_argument("--first", action="store_true")
args = ap.parse_args()
test_mods = args.mod or [Path(__file__).resolve().parent / "mp-test"]
from factorio_paths import PATHS, run_dir  # noqa: E402
HERE = Path(__file__).resolve().parent
# (fresh folders: a game killed mid-write can leave a config.ini the next start asks to reset)
for n in ("mp-server", "mp-client", "mp-plain"):
    shutil.rmtree(HERE / "run" / n, ignore_errors=True)
S, C, P = (run_dir(HERE / "run" / n) for n in ("mp-server", "mp-client", "mp-plain"))
# (test games run without Steam (FSE_NO_STEAM): each client's name comes from its own player-data.json)
for d, name in ((C, "fse-test-client"), (P, "fse-test-plain")):
    (d / "player-data.json").write_text(json.dumps({"service-username": name}))
MODS = HERE / "run" / "mp-mods"
LAUNCH = str(NATIVE / "dist" / "fse-launcher.exe")
PORT = "34297"

shutil.rmtree(MODS, ignore_errors=True)
MODS.mkdir(parents=True)
shutil.copytree(NATIVE / "mods" / "fse-std", MODS / "fse-std")
for m in test_mods:
    shutil.copytree(m, MODS / m.name, ignore=shutil.ignore_patterns("test", "__pycache__", "*.zip"))
names = ["base", "fse-std"] + [m.name for m in test_mods]
(MODS / "mod-list.json").write_text(json.dumps({"mods": [{"name": n, "enabled": n in names} for n in
                                                         names + ["elevated-rails", "quality", "space-age"]]}))
save = S / "mp.zip"
save.unlink(missing_ok=True)
settings = S / "server-settings.json"
cfg = json.loads((PATHS["game_data"] / "server-settings.example.json").read_text(encoding="utf-8"))
cfg.update({"name": "fse-mp-test", "description": "", "visibility": {"public": False, "lan": False},
            "require_user_verification": False, "autosave_interval": 0, "auto_pause": False, "username": "",
            "password": "", "token": "", "game_password": ""})
settings.write_text(json.dumps(cfg))
env = {k: v for k, v in os.environ.items() if k != "FSE_OFF"}
common = ["--mod-directory", str(MODS)]
subprocess.run([LAUNCH, "--config", str(S / "config.ini"), *common, "--create", str(save)], capture_output=True, env=env)

procs = []


def log(d):
    f = d / "factorio-current.log"
    return f.read_text(errors="replace") if f.exists() else ""


def wait(cond, secs):
    end = time.time() + secs
    while time.time() < end:
        if cond():
            return True
        time.sleep(1)
    return False


def out(d, name):
    f = d / "script-output" / name
    return f.read_text().splitlines() if f.exists() else []


try:
    server = subprocess.Popen([LAUNCH, "--config", str(S / "config.ini"), *common, "--start-server", str(save),
                               "--server-settings", str(settings), "--port", PORT], env=env,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    procs.append(server)
    if not wait(lambda: "changing state from(CreatingGame) to(InGame)" in log(S), 120):
        sys.exit("the server didn't start:\n" + log(S)[-2000:])
    client = subprocess.Popen([LAUNCH, "--config", str(C / "config.ini"), *common, "--mp-connect", f"127.0.0.1:{PORT}"],
                              env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    procs.append(client)
    joined = wait(lambda: len(out(C, "mp-player-1.txt")) >= 4, 240)
    client.kill()
    s_lines, c_lines = out(S, "mp-server.txt"), out(C, "mp-player-1.txt")
    print("server:", s_lines[-3:], "\nclient:", c_lines[-3:])
    desync = [l for d in (S, C) for l in log(d).splitlines() if re.search("desync", l, re.I)]
    # (the same tick must read the same on both peers; the client starts later)
    common_ticks = {l.split()[1]: l for l in s_lines}
    same = [l == common_ticks.get(l.split()[1], l) for l in c_lines]
    synced = bool(c_lines) and re.search(args.expect, c_lines[-1]) is not None
    print("joined:", joined, " same state at the same ticks:", all(same) and len(same) > 0, " expected:", synced,
          "\ndesync lines:", desync[:5] or "none")
    ok1 = joined and all(same) and len(same) > 0 and synced and not desync

    if args.first or args.no_kick:
        print("PASS" if ok1 else "FAIL")
        sys.exit(0 if ok1 else 1)
    # a client without fse: kicked by the handshake (once the server has let the first one go: same player name)
    left = wait(lambda: "removing peer(1)" in log(S), 120)
    print("first client gone:", left)
    plain_env = dict(env, FSE_OFF="1", SteamAppId="427520", SteamGameId="427520")
    plain = subprocess.Popen([str(PATHS["factorio_exe"]), "--config", str(P / "config.ini"), *common, "--mp-connect",
                              f"127.0.0.1:{PORT}"], env=plain_env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    procs.append(plain)
    kicked = wait(lambda: "FSE: kicking player" in log(S), 180)
    print("plain client kicked:", kicked)
    print("\n".join(l for l in (log(S) + log(P)).splitlines() if "FSE: kicking" in l)[-800:])
    ok = ok1 and kicked
    print("PASS" if ok else "FAIL")
    sys.exit(0 if ok else 1)
finally:
    for p in procs:
        p.kill()
