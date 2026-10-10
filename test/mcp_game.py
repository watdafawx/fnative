"""MCP server (stdio, no dependencies): a warm headless Factorio that an agent tests mods in.

One headless server (its own write-data folder, test/run/mcp-game) with a mod and optionally one of its test cases,
driven over the game's own RCON. The game is held at speed 0.01 (a server can't pause); time moves through `ticks`.
Tools:

  start(mod, case?, with?, base?, fse?)   map created, server up (~3 s); a running one is replaced
  lua(code, mod?)        code run as a function body in mod's own Lua state (`/sc __mod__`), the return value as text
  reload()               the game saved, the mods copied again from their folders, the save loaded (~3 s, state kept:
                         control.lua and data.lua changes both; game.reload_script does nothing on a server)
  ticks(n)               the game runs about n ticks as fast as it can, then is held again; new log errors in the answer
  log(errors?, lines?)   the game's log (or only its error lines)
  output(name?)          a script-output file (helpers.write_file), or the list of them
  stop()

Registered in the dev folder's .mcp.json as "factorio". Check: test/run_mcp_game.py.
"""
import atexit
import json
import os
import re
import secrets
import shutil
import socket
import struct
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from factorio_paths import BASE_MODS, PATHS, run_dir, stage_mods, user_mod  # noqa: E402

RUN = HERE / "run" / "mcp-game"
LAUNCHER = HERE.parent / "dist" / "fse-launcher.exe"
ERROR = re.compile(r"Error|non-recoverable")
HELD = "/sc game.speed = 0.01"


class Rcon:
    """the Source RCON protocol Factorio speaks: auth once, then one command, one answer"""

    def __init__(self, port, password):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=60)
        self.id = 0
        self.send(3, password)
        while True:
            rid, kind, _ = self.recv()
            if kind == 2:  # (the auth answer; id -1 when the password is wrong)
                if rid == -1:
                    raise RuntimeError("rcon: wrong password")
                return

    def send(self, kind, body):
        self.id += 1
        data = body.encode("utf-8") + b"\0\0"
        self.sock.sendall(struct.pack("<iii", len(data) + 8, self.id, kind) + data)

    def exact(self, n):
        buf = b""
        while len(buf) < n:
            part = self.sock.recv(n - len(buf))
            if not part:
                raise ConnectionError("rcon: the game closed the connection")
            buf += part
        return buf

    def recv(self):
        (size,) = struct.unpack("<i", self.exact(4))
        rid, kind = struct.unpack("<ii", self.exact(8))
        return rid, kind, self.exact(size - 8)[:-2].decode("utf-8", "replace")

    def command(self, text):
        self.send(2, text)
        return self.recv()[2]


class Game:
    def __init__(self):
        self.proc = self.rcon = None
        self.mods = {}
        self.common = self.env = None
        self.log_seen = 0

    def log_text(self):
        f = RUN / "factorio-current.log"
        return f.read_text(encoding="utf-8", errors="replace") if f.exists() else ""

    def errors_note(self):
        text = self.log_text()
        new, self.log_seen = text[self.log_seen:], len(text)
        errors = [l for l in new.splitlines() if ERROR.search(l)]
        return ("\nerrors in the log:\n" + "\n".join(errors[-30:])) if errors else ""

    def start(self, mod, case=None, base=None, fse=False, **kw):
        self.stop()
        mod = Path(mod).resolve()
        if not (mod / "info.json").exists():
            raise ValueError(f"{mod} has no info.json")
        self.mods = {mod.name: mod, **{n: user_mod(n) for n in kw.get("with") or []}}
        if case:
            self.mods[case] = mod / "test" / case
        if fse:
            self.mods["fse-std"] = HERE.parent / "mods" / "fse-std"
        run = run_dir(RUN)
        md = run / "mcp-mods"
        stage_mods(md, self.mods, BASE_MODS if base is None else base)
        self.env = {k: v for k, v in os.environ.items() if not (fse and k == "FSE_OFF")}
        self.common = [str(LAUNCHER if fse else PATHS["factorio_exe"]), "--config", str(run / "config.ini"),
                       "--mod-directory", str(md)]
        save = run / "mcp.zip"
        save.unlink(missing_ok=True)
        p = subprocess.run(self.common + ["--create", str(save)], capture_output=True, text=True, encoding="utf-8",
                           errors="replace", env=self.env)
        if p.returncode or not save.exists():
            raise RuntimeError("the map wasn't created:\n" + p.stdout[-3000:])
        self.launch(save)
        return f"running {', '.join(self.mods)} at tick {self.lua('return game.tick')}, held" + self.errors_note()

    def launch(self, save):
        settings = RUN / "server-settings.json"
        cfg = json.loads((PATHS["game_data"] / "server-settings.example.json").read_text(encoding="utf-8"))
        cfg.update({"name": "mcp-game", "description": "", "visibility": {"public": False, "lan": False},
                    "require_user_verification": False, "autosave_interval": 0, "auto_pause": False,
                    "username": "", "password": "", "token": "", "game_password": ""})
        settings.write_text(json.dumps(cfg))
        password = secrets.token_hex(8)
        with socket.socket() as s:  # (two free ports: RCON's and the game's UDP one)
            s.bind(("127.0.0.1", 0))
            rcon_port = s.getsockname()[1]
        with socket.socket(type=socket.SOCK_DGRAM) as s:
            s.bind(("127.0.0.1", 0))
            game_port = s.getsockname()[1]
        self.log_seen = 0
        self.proc = subprocess.Popen(
            self.common + ["--start-server", str(save), "--server-settings", str(settings), "--port", str(game_port),
                           "--bind", "127.0.0.1", "--rcon-bind", f"127.0.0.1:{rcon_port}", "--rcon-password", password],
            env=self.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            creationflags=subprocess.BELOW_NORMAL_PRIORITY_CLASS)
        end = time.time() + 180
        while "changing state from(CreatingGame) to(InGame)" not in self.log_text():
            if self.proc.poll() is not None or time.time() > end:
                text = self.log_text()
                self.stop()
                raise RuntimeError("the server didn't start:\n" + text[-3000:])
            time.sleep(0.2)
        self.rcon = Rcon(rcon_port, password)
        self.command(HELD)

    def command(self, text):
        if not self.rcon:
            raise RuntimeError("no game running: start first")
        return self.rcon.command(text).rstrip("\n")

    def lua(self, code, mod=None):
        wrapped = (f"local ok, r = pcall(function() {code}\nend) "
                   "rcon.print(ok and (type(r) == 'string' and r or serpent.line(r, {comment = false, nocode = true}))"
                   " or ('error: ' .. tostring(r)))")
        return self.command(f"/sc {f'__{mod}__ ' if mod else ''}{wrapped}")

    def reload(self):
        tick = self.lua("return game.tick")
        save = RUN / "mcp.zip"
        seen = len(self.log_text())
        self.command("/server-save")
        end = time.time() + 60
        while "Saving finished" not in self.log_text()[seen:]:
            if time.time() > end:
                raise RuntimeError("the game didn't save")
            time.sleep(0.1)
        self.stop()
        md = RUN / "mcp-mods"
        for name, src in self.mods.items():
            if src.is_dir():
                shutil.rmtree(md / name, ignore_errors=True)
                shutil.copytree(src, md / name, ignore=shutil.ignore_patterns("test", ".git", "__pycache__"))
        self.launch(save)
        return f"reloaded at tick {tick}, now {self.lua('return game.tick')}, held" + self.errors_note()

    def ticks(self, n):
        goal = int(self.lua("return game.tick")) + n
        self.command("/sc game.speed = 100")
        while True:
            t = int(self.lua("return game.tick"))
            if t >= goal or self.proc.poll() is not None:
                break
            time.sleep(0.02 if goal - t < 600 else 0.2)
        self.command(HELD)
        return f"tick {t}" + self.errors_note()

    def log(self, errors=False, lines=60):
        text = self.log_text().splitlines()
        if errors:
            text = [l for l in text if ERROR.search(l)]
        return "\n".join(text[-lines:]) or "(nothing)"

    def output(self, name=None):
        out = RUN / "script-output"
        if not name:
            return "\n".join(sorted(str(p.relative_to(out)) for p in out.rglob("*") if p.is_file())) or "(none)"
        return (out / name).read_text(encoding="utf-8", errors="replace")

    def stop(self):
        if self.rcon:
            self.rcon.sock.close()
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()
        self.proc = self.rcon = None
        return "stopped"


GAME = Game()
atexit.register(GAME.stop)


def schema(props, required=()):
    return {"type": "object", "properties": props, "required": list(required)}


S, I, B = {"type": "string"}, {"type": "integer"}, {"type": "boolean"}
TOOLS = {
    "start": ("Start a headless Factorio server with a mod folder and optionally one of its test cases "
              "(<mod>/test/<case>, a scenario mod). Replaces a running one. The game is held (speed 0.01) until "
              "`ticks`. Base mods: base, elevated-rails, quality, space-age unless `base` is given (e.g. [\"base\"]). "
              "`with`: more mods by name from the user's own mods folder (newest zip). `fse`: start through FSE's "
              "launcher (fse/dist) with fse-std.",
              schema({"mod": {**S, "description": "absolute path of the mod folder (has info.json)"},
                      "case": S, "with": {"type": "array", "items": S}, "base": {"type": "array", "items": S},
                      "fse": B}, ["mod"])),
    "lua": ("Run Lua on the game thread as a function body; `return x` gives x (serpent.line for tables, errors as "
            "'error: ...'). With `mod`, it runs in that mod's own Lua state (its globals and storage; control.lua's "
            "locals are not visible). Without, in the scenario's.",
            schema({"code": S, "mod": S}, ["code"])),
    "reload": ("After editing a mod: save, copy the mods again from their folders, load the save (~3 s, game state "
               "kept). Covers control.lua and data.lua. Tick and new log errors in the answer.", schema({})),
    "ticks": ("Run about n game ticks as fast as possible (60 ticks = 1 s of game time), then hold. Answers the tick "
              "and any new errors in the log.", schema({"n": I}, ["n"])),
    "log": ("The game's log, last `lines` lines (60), or only lines with errors.",
            schema({"errors": B, "lines": I})),
    "output": ("A file from the game's script-output (helpers.write_file), or the list of them without a name.",
               schema({"name": S})),
    "stop": ("Stop the game.", schema({})),
}


def call(name, args):
    if name == "start":
        return GAME.start(**args)
    if name == "lua":
        return GAME.lua(args["code"], args.get("mod"))
    if name == "ticks":
        return GAME.ticks(args["n"])
    if name == "log":
        return GAME.log(args.get("errors", False), args.get("lines", 60))
    if name == "output":
        return GAME.output(args.get("name"))
    if name in ("reload", "stop"):
        return getattr(GAME, name)()
    raise ValueError(f"no tool {name}")


def serve():
    for line in sys.stdin:
        msg = json.loads(line)
        if "id" not in msg:
            continue  # (notifications)
        method, params = msg.get("method"), msg.get("params") or {}
        reply = {"jsonrpc": "2.0", "id": msg["id"]}
        if method == "initialize":
            reply["result"] = {"protocolVersion": params.get("protocolVersion", "2025-06-18"),
                               "capabilities": {"tools": {}}, "serverInfo": {"name": "factorio", "version": "0.1.0"}}
        elif method == "tools/list":
            reply["result"] = {"tools": [{"name": n, "description": d, "inputSchema": s}
                                         for n, (d, s) in TOOLS.items()]}
        elif method == "tools/call":
            try:
                text, err = str(call(params["name"], params.get("arguments") or {})), False
            except Exception as e:  # (an answer the agent can read beats a dead server)
                text, err = f"{type(e).__name__}: {e}", True
            reply["result"] = {"content": [{"type": "text", "text": text}], "isError": err}
        elif method == "ping":
            reply["result"] = {}
        else:
            reply["error"] = {"code": -32601, "message": f"no method {method}"}
        print(json.dumps(reply), flush=True)


if __name__ == "__main__":
    serve()
