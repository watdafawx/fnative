"""mcp_game.py end to end over stdio, as an MCP client starts it: a probe mod (made here, under test/run) started,
Lua in its own state, ticks, an edit picked up by reload with storage kept, an error answered, stop."""
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
PROBE = HERE / "run" / "mcp-probe"
PROBE.mkdir(parents=True, exist_ok=True)
(PROBE / "info.json").write_text(json.dumps({"name": "mcp-probe", "version": "0.1.0", "title": "mcp probe",
                                             "author": "fse", "factorio_version": "2.0", "dependencies": ["base"]}))
(PROBE / "control.lua").write_text("VERSION = 1\n")

srv = subprocess.Popen([sys.executable, str(HERE / "mcp_game.py")], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                       text=True, encoding="utf-8")
ids = iter(range(1, 1000))
failed = []


def rpc(method, params=None):
    srv.stdin.write(json.dumps({"jsonrpc": "2.0", "id": next(ids), "method": method, "params": params or {}}) + "\n")
    srv.stdin.flush()
    return json.loads(srv.stdout.readline())


def tool(tool_name, **args):
    r = rpc("tools/call", {"name": tool_name, "arguments": args})["result"]
    return r["content"][0]["text"], r["isError"]


def check(name, ok, got):
    print(("PASS " if ok else "FAIL ") + name + " :: " + str(got).replace("\n", " | ")[:200])
    if not ok:
        failed.append(name)


try:
    init = rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t"}})
    check("initialize", init["result"]["serverInfo"]["name"] == "factorio", init["result"])
    names = [t["name"] for t in rpc("tools/list")["result"]["tools"]]
    check("tools listed", names == ["start", "lua", "reload", "ticks", "log", "output", "stop"], names)
    text, err = tool("start", mod=str(PROBE), base=["base"])
    check("started", not err and "mcp-probe" in text, text)
    text, _ = tool("lua", mod="mcp-probe", code="storage.kept = 7\nreturn {VERSION, script.mod_name}")
    check("lua in the mod's state", text == '{1, "mcp-probe"}', text)
    text, _ = tool("ticks", n=600)
    tick = int(text.split()[1])
    check("ticks", 600 <= tick < 1500, text)
    (PROBE / "control.lua").write_text("VERSION = 2\n")
    text, err = tool("reload")
    check("reload", not err, text)
    text, _ = tool("lua", mod="mcp-probe", code="return {VERSION, storage.kept, game.tick >= %d}" % tick)
    check("the edit loaded, storage and time kept", text == "{2, 7, true}", text)
    text, _ = tool("lua", code="return nope.x")
    check("a Lua error answered", text.startswith("error:") and "nope" in text, text)
    text, err = tool("output", name="missing.txt")
    check("a tool error answered", err, text)
finally:
    print(tool("stop")[0])
    srv.stdin.close()
    srv.wait(30)
print("ALL PASS" if not failed else "FAILED: " + ", ".join(failed))
sys.exit(1 if failed else 0)
