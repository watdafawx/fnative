# FSE (Factorio Script Extender)

A native extension layer for Factorio 2.0 on Windows. Native code (Rust, C, anything with a C ABI, and Python) runs
inside the game and Lua mods reach it through a global `native` table. With it, mods can do things the Lua API
can't: read the mouse, use the clipboard, run work on threads, call Python libraries, show web pages over the game,
profile engine functions and patch small engine bugs.

Installing is unzipping into the game's folder; uninstalling is deleting what that added. No launcher, no Steam
launch options: a small `version.dll` beside `factorio.exe` (Windows loads it from the game's folder) loads
`fse.dll` before the game starts. fse finds the engine's functions by name in `factorio.pdb`, the debug symbols Wube
ships with the game, so game updates need no new offsets, and Steam updates leave the install alone.

> **Single player only.** Native results aren't part of the deterministic simulation, so they would desync
> multiplayer games and replays. Windows only. Not for the mod portal (it can't carry native binaries).
> Keep crash-report uploading off while you experiment: crashes with foreign code in the process are noise for Wube.

## Getting started

### 1. Install

You need Windows 10/11 and Factorio 2.0 (Steam or standalone). For the `py` plugin, also
[Python](https://www.python.org) 3.10 to 3.13 on `PATH`.

1. Download `fse-<version>-windows.zip` from [Releases](https://github.com/watdafawx/fse/releases).
2. Open the game's folder. Steam: Library → right-click Factorio → Manage → Browse local files.
3. Extract the zip there. It adds `bin\x64\version.dll` (the loader) and an `fse` folder.
4. Start the game as always.

At the first start fse puts its mods into your mods folder (newer versions too, at later starts):

| mod | what it does |
|---|---|
| `fse-std` | GUI library for other mods: movable, resizable windows that remember their place, and drag & drop. Works without fse too (no resize grip; click to pick, click to drop). |
| `fse-hub` | an **F** button (top left) with tabs: loader and plugins, your mods (added/updated dates, real load order, portal updates), the start-up time report per mod, the engine profiler. Needs `fse-std`. |
| `fse-bridge` | runs commands from the local web API on the game thread. |
| `fse-agent` | characters without a player that an external program (an AI agent, `examples/agent_client.py`) can drive. |

Every mod checks for fse first and does nothing (or falls back) without it.

Settings go in `fse\fse.env` (optional, see the comments in it). The log is `fse\fse.log`. A good start:

```
fse 0.6.0 loading
105032 engine symbols read in 61.48ms
game build EAAB184A...-1 (known)
plugin ...\fse_std.dll: init ok
ready in 70.97ms
```

**Off for one start:** set `FSE_OFF=1` in the environment. **Uninstall:** run `fse\uninstall.cmd`, or delete
`bin\x64\version.dll` and the `fse` folder. The mods stay; disable them in the game if you like.

### 2. Build from source

Needs [Rust](https://rustup.rs) (stable, MSVC toolchain) and Python. Optional: `clang-cl` (LLVM) for the C example
plugin, the WebView2 runtime (part of Windows 11) for pages in a panel over the game (else they open in your browser).

```
git clone https://github.com/watdafawx/fse
cd fse
python build.py
python install.py
```

`build.py` puts everything in `dist\`, laid out like the install: `bin\x64\version.dll` (the loader), `fse.dll`
(the core), `plugins\*.dll` (`py`, `std`, `web`, `profiler`, `fixes`, `diag`, `entityinfo`, and `hello` if clang-cl
was found), `py\`, `mods\`, `web\` and `fse.env` (created once, never overwritten). `install.py` copies the loader
into the game and makes the game's `fse` folder a junction to `dist\`, so each rebuild is live at the next start;
`install.py --remove` undoes it. `package.py` makes the release zip.

`dist\fse.exe` is the launcher the tests use: it starts the game and injects `fse.dll` (from beside itself) with
nothing installed. Pass it any Factorio arguments; it finds the game in the usual Steam libraries, or set
`FACTORIO_EXE`.

### 3. Check it works

In game, open the console and run:

```
/c game.print(native and native.version() or "no loader")
```

Or click the **F** button if you installed `fse-hub`.

## Settings (`fse\fse.env`)

One `KEY=value` per line, read at every start (by the loader, or the launcher beside `dist\fse.exe`).

| key | default | does |
|---|---|---|
| `FACTORIO_EXE` | the first Steam library that has the game | the game the launcher starts |
| `FACTORIO_MODS` | the game's own (`--mod-directory`, else its config) | the mods folder fse's mods go into, and the mod manager and start-up report read |
| `FSE_PYPATH` | | more folders with Python modules Lua can call, `;`-separated (fse's `py` folder is always there) |
| `FSE_PLUGINS` | `fse\plugins` | where plugins are loaded from; empty loads none |
| `FSE_LOG` | `fse\fse.log` | the log file |
| `FSE_WEB_PORT` | 8790 | the local web API's port (it takes the first free one of 8790-8799) |
| `FSE_PANEL=0` | | open pages in the browser instead of the panel over the game |
| `FSE_OVERLAY=0` | | no hub buttons over the main menu (to hide just one, use the dashboard's *Main menu buttons*) |
| `FSE_STD_WHEEL=0` | | don't watch the mouse wheel (the `std` plugin's wheel events) |
| `FSE_FIXES=0`, `FSE_FIX_<NAME>=0` | | turn off all engine fixes, or one |
| `FSE_DIAG=1` | | log every engine error the game raises, even ones it catches itself |

## Using it from a mod

`native` exists in every Lua state (settings, data and control stage of every mod) when fse is installed
(or the game was started through the launcher). Always check for it, so your mod still works without it:

```lua
if native then
  native.log("hello from " .. script.mod_name)
  local out, err = native.call("std", "clipboard_get")
end
```

| function | does |
|---|---|
| `native.version()` | the core's version |
| `native.log(text)` | a line in fse.log |
| `native.plugins()` | `{ plugin = { function names } }` |
| `native.call(plugin, fn, input?)` | runs on the game thread; returns the output string, or `nil, error` |
| `native.start(plugin, fn, input?)` | runs on a worker thread (for functions registered as thread-safe); returns a job id |
| `native.poll(id)` | `"pending"`, `"done", output` or `"error", message`; a result is handed out once |
| `native.to_json(value, skip?)` | any Lua value as JSON, much faster than `helpers.table_to_json`; works in the data stage too (`data.raw`); `skip = {key = true}` leaves keys out at any depth |
| `native.symbols(part, limit?)` | engine function names containing `part` |
| `native.build()` | `{ build = "...", new = true }` on the first start of a new game build |

Rules: native functions never raise Lua errors; they return `nil, message`. Inputs and outputs are strings (JSON by
convention). Long work goes through `start`/`poll` so a tick never waits.

### Python

Put a module on `FSE_PYPATH` with functions that take a string and return a string:

```python
# mytool.py
import json
def double(s):
    return json.dumps([x * 2 for x in json.loads(s)])
```

```lua
local out = native.call("py", "mytool:double", "[1, 2, 3]")    -- "[2, 4, 6]"
```

The interpreter starts on the first call (20-40 ms) and stays, so module state persists and big data loads once.
`native.call("py", "_reload", "mytool")` reloads a module while the game runs. `print` goes to fse.log.
Python exceptions come back as `nil, traceback`.

### The `fse-std` library

```lua
local window = require("__fse-std__/window")
local events = require("__fse-std__/events")
local safe   = require("__fse-std__/safe")
```

| module | gives |
|---|---|
| `window` | `create(player, {name, title, width, height, resizable, close, min_width, min_height})` → frame, content: a title bar to move it, a close button, a corner grip to resize; position and size remembered per player; `on_close`, `on_resize`, `size` |
| `dnd` | `draggable(el, payload)`, `droppable(el, accept)`, `on_drop(fn)`: press, drag (a ghost follows the cursor, the target lights up), release |
| `input` | the cursor and mouse buttons (`read()`), the wheel, clipboard, real time in ms, reading files under script-output |
| `events` | `events.register({...})`: one registration per event for all of the above plus your own handlers, each guarded |
| `safe` | `guard(name, fn)` (an error is logged instead of crashing the game), `has(plugin)`, `log`, `chain` |

Add `"? fse-std"` to your mod's dependencies if it should work without it, and check
`script.active_mods["fse-std"]` before requiring.

## Plugins

| plugin | gives |
|---|---|
| `py` | Python functions from Lua (above) |
| `std` | mouse position and buttons in GUI pixels, the wheel (with a lease so the game camera doesn't zoom while your GUI uses it), modifier keys, window size and focus, clipboard, real time, opening http(s) links |
| `web` | a local HTTP API and pages on `127.0.0.1` (token in `fse\web-token.txt`): status, the profiler, native calls, `remote.call` through `fse-bridge`; `web.open(page)` shows a page in a panel over the game |
| `profiler` | times engine functions by name (`fse\plugins\profiler.json`): Game::update, entity updates by kind, belts, electric networks, robots, pathfinder, Lua events. Only between `profiler.start` and `profiler.stop` |
| `fixes` | small engine bug fixes, each checking the exact bytes it expects first and skipping itself (logged) if a game update changed them |
| `diag` | off unless `FSE_DIAG=1`: logs the message and stack of every engine error |
| `entityinfo` | a mod's own rows in the game's info panel for the entity under the cursor (below) |
| `hello` | the C example |

### Engine fixes

- **data-cache-tilde**: with `[other] cache-prototype-data=true` in config.ini, Factorio can skip every mod's data
  stage at start. The cache never loads if any enabled mod has a `~` dependency: the cache file doesn't store that
  flag but the check compares it. The fix leaves that flag out of the check. On a 500-mod pack, starts went from 90 s
  to 50 s. Details in [docs/bug-report-data-cache.md](docs/bug-report-data-cache.md).
- **gc-idle-skip**: every tick the engine forces a Lua GC step in each mod's Lua state, even states that allocated
  nothing since their last cycle, so they re-walk their whole heap again and again. The fix skips the step only for
  a state that is between cycles and whose memory hasn't changed. With ~400 mods this halved the GC time per tick.

### Rows in the game's entity info panel (`entityinfo`)

The panel under the minimap that describes the entity under the cursor is the engine's own; no mod API reaches it.
`entityinfo` lets a mod add rows to it for an entity of its choice, in the game's look (rich text works: `[item=...]`,
`[color=...]`):

```lua
native.call("entityinfo", "set", helpers.table_to_json({ name = ent.name, unit = ent.unit_number,
  rows = { { "Doing", "building" }, { "Weapon", "[item=submachine-gun] range 18" } } }))
native.call("entityinfo", "clear", helpers.table_to_json({ unit = ent.unit_number }))   -- or "{}" for all
native.call("entityinfo", "status", "")   -- {"hooked": [...], "entities": n, "shown": n}
```

The panel is rebuilt every frame while the entity is hovered, so rows a mod sets again (say every 30 ticks, from
`on_selected_entity_changed` on) stay current. Only entities with a unit number (anything with an owner: buildings,
characters, vehicles) can have rows. Without the loader `native` is nil: keep a Lua fallback (AI Crew shows its own
card then). How: the virtual `addToDescription` of Entity, EntityWithHealth, EntityWithOwner, Character, Car and
SpiderVehicle is hooked (not every override calls its base); after the outermost one ran, the rows go in through
`Description::add`, the call the engine's own rows use.

## How it works

```
version.dll   the loader, beside factorio.exe: forwards to the system's version.dll; points the game's entry
                  point at itself, and there loads fse.dll and waits until it is ready before the game starts
(fse.exe      the launcher: starts factorio.exe suspended, injects fse.dll, waits, resumes the game)
fse.dll (Rust)    reads the function symbols of factorio.pdb (~100k functions, 60 ms)
                      finds the game's Lua 5.2 C API by name and hooks luaopen_base:
                      every Lua state gets a global `native` table
                      loads the plugins (include/fse.h) from the plugins folder
```

A game that restarts itself (after a mod change) loads fse again by itself. The launcher keeps the game in a job
object, so closing the launcher ends the game too, and starts a restarting game through itself.

**Game updates.** At every start, before anything is hooked:

1. factorio.exe's build id must match factorio.pdb's. If they differ (a half-applied update), nothing is hooked, the
   game runs plain and the log says why.
2. Functions are read fresh from the pdb, so new addresses need nothing done.
3. What our code relies on (`needs\*.needs.json`, `plugins\<plugin>.needs.json`: functions, classes and fields) is
   checked. A missing core function stops the core; a plugin missing something is the only one turned off.
4. On the first start of a new build, `fse\cache\<build>\report.txt` lists what moved since the previous build.

Plugins never hard-code engine offsets: they ask the host (`field_offset("Inserter", "heldStack")`). `fse-pdb`
(in `target\release`) explores a build from the command line: `info`, `functions Inserter::`, `classes Transport`,
`class Inserter`, `report`.

## Writing a plugin

A plugin is a DLL exporting `fse_plugin_init(host)` that registers functions taking and returning strings. See
[include/fse.h](include/fse.h) and [plugins/hello-c/hello.c](plugins/hello-c/hello.c) (40 lines). Rust
plugins share `crates/fse-plugin` (the host table and an `export!` macro); `crates/fse-std` is a complete
example. Put the DLL in `fse\plugins`.

## Tests

The scripts under `test\` start a headless or real game with their own write-data folder, so they never touch your
saves or mods. They find the game like the launcher does (`FACTORIO_EXE` to override).

| script | checks |
|---|---|
| `run_demo.py` | `native` in every stage, plugins, errors, worker jobs, call cost, Python |
| `run_build_check.py` | the build report, a known build, a faked update's diff, a mismatched pdb refused |
| `run_fixes.py` | the data cache fix with a `~` dependency mod |
| `run_web.py` | the web API, bridge and agents end to end |
| `run_std_gui.py` | `fse-std` drag & drop and resizing in a real game window, with screenshots |
| `run_hub.py` | the menu overlay, the hub and its tabs in a real game window |

## Related

- [bpgen](https://github.com/watdafawx/bpgen): a production-line blueprint planner that runs inside the game through
  FSE and previews the blueprint with the game's own renderer.
- Prior art: Rivets (Rust, DLL injection and pdb symbols, Factorio 1.1).

## License

MIT, see [LICENSE](LICENSE).
