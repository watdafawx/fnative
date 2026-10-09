"""The smallest agent: drives an fse-agent character over the fse web API. Replace `decide` with an AI.

    python native/examples/agent_client.py [name]

Needs FSE installed (or the game started through dist/fse-launcher.exe) with the fse-bridge and fse-agent mods enabled.
The token comes from web-token.txt beside the launcher (native/dist).
"""
import json
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

DIST = Path(__file__).resolve().parents[1] / "dist"
BASE = "http://127.0.0.1:8790"
TOKEN = (DIST / "web-token.txt").read_text().strip()


def call(interface, function, *args):
    """a remote interface function, run on the game thread; its result (raises on a game-side error)"""
    body = json.dumps({"interface": interface, "function": function, "args": list(args)}).encode()
    req = urllib.request.Request(BASE + "/api/game", data=body, method="POST",
                                 headers={"Authorization": f"Bearer {TOKEN}"})
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return json.loads(r.read())["result"]
    except urllib.error.HTTPError as e:
        raise RuntimeError(json.loads(e.read()).get("error", str(e))) from None


def decide(status, seen, events):
    """one step of the agent's mind: the world in, an action out. Here: walk to the nearest ore and say so."""
    if any(e["kind"] == "arrived" for e in events):
        return ("say", "I'm at the ore")
    if status.get("walking_to"):
        return None
    ores = [e for e in seen["entities"] if e["type"] == "resource"]
    if seen["resources"]:
        # (look lists resources by total amount; walk toward the resource-rich spot: here, just a step east)
        p = status["position"]
        return ("walk_to", p["x"] + 4, p["y"])
    return ("say", "nothing to mine around here") if not ores else None


def main(name="ada"):
    if name not in (call("agent", "list") or {}):
        print("spawned", call("agent", "spawn", name))
    for step in range(10):
        status = call("agent", "status", name)
        seen = call("agent", "look", name, 12)
        events = call("agent", "events", name) or []
        action = decide(status, seen, events)
        print(f"step {step}: at {status['position']} resources {seen['resources']} events {[e['kind'] for e in events]} -> {action}")
        if action:
            call("agent", action[0], name, *action[1:])
        time.sleep(1)


if __name__ == "__main__":
    main(*sys.argv[1:])
