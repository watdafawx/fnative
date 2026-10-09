# fnative

A native extension layer for Factorio 2.0 on Windows. Native code (Rust, C, anything with a C ABI, and Python) runs
inside the game and Lua mods reach it through a global `native` table. With it, mods can do things the Lua API
can't: read the mouse, use the clipboard, run work on threads, call Python libraries, show web pages over the game,
profile engine functions and patch small engine bugs.

The game install is never changed. A launcher starts the game, loads `fnative.dll` into it at start-up and finds the
engine's functions by name in `factorio.pdb`, the debug symbols Wube ships with the game. Game updates need no new
offsets.

> **Single player only.** Native results aren't part of the deterministic simulation, so they would desync
> multiplayer games and replays. Windows only. Not for the mod portal (it can't carry native binaries).
> Keep crash-report uploading off while you experiment: crashes with foreign code in the process are noise for Wube.

## Getting started

### 1. What you need

- Windows 10/11 and Factorio 2.0 (Steam or standalone). `bin\x64\factorio.pdb` must be next to `factorio.exe`; the
  Windows builds include it.
- [Rust](https://rustup.rs) (stable, MSVC toolchain).
- [Python](https://www.python.org) 3.10 to 3.13, on `PATH`. It runs the build script and is what the `py` plugin
  embeds.
- Optional: `clang-cl` (LLVM), only for the C example plugin. The build skips it otherwise.
- Optional: the WebView2 runtime (part of Windows 11) for pages shown in a panel over the game. Without it, pages
  open in your browser.

### 2. Build

```
git clone https://github.com/watdafawx/fnative
cd fnative
python build.py
```

Everything goes to `dist\`:

| file | what it is |
|---|---|
| `factorio-native.exe` | the launcher |
| `fnative.dll` | the core, loaded into the game |
| `plugins\*.dll` | the plugins (`py`, `std`, `web`, `profiler`, `fixes`, `diag`, and `hello` if clang-cl was found) |
| `fnative.env` | your settings, created on the first build and never overwritten |

### 3. Start the game through the launcher

Steam: Library → Factorio → Properties → Launch options:

```
"C:\path\to\fnative\dist\factorio-native.exe" %COMMAND%
```

Or run `dist\factorio-native.exe` directly with any Factorio arguments. It finds the game in the usual Steam
libraries; if yours is elsewhere, set `FACTORIO_EXE` in `dist\fnative.env`. Starting the game without the launcher
gives you the plain game as always.

The log is `dist\fnative.log`. A good start looks like this:

```
fnative 0.5.0 loading
105032 engine symbols read in 61.48ms
game build EAAB184A...-1 (known)
plugin ...\fnative_std.dll: init ok
ready in 70.97ms
```

### 4. Install the mods

Copy (or symlink) the folders under `mods\` that you want into your Factorio mods folder
(`%APPDATA%\Factorio\mods`):

| mod | what it does |
|---|---|
| `fnative-std` | GUI library for other mods: movable, resizable windows that remember their place, and drag & drop. Works without the loader too (no resize grip; click to pick, click to drop). |
| `fnative-hub` | an **F** button (top left) with tabs: loader and plugins, your mods (added/updated dates, real load order, portal updates), the start-up time report per mod, the engine profiler. Needs `fnative-std`. |
| `fnative-bridge` | runs commands from the local web API on the game thread. |
| `fnative-agent` | characters without a player that an external program (an AI agent, `examples/agent_client.py`) can drive. |
| `fnative-std-demo`, `native-demo` | examples: `/stddemo` opens a window of items to reorder by dragging. |

Every mod checks for the loader first and does nothing (or falls back) when the game wasn't started through it.

### 5. Check it works

In game, open the console and run:

```
/c game.print(native and native.version() or "no loader")
```

Or click the **F** button if you installed `fnative-hub`.

## Settings (`dist\fnative.env`)

One `KEY=value` per line. The launcher reads it at every start and passes it to the game.

| key | default | does |
|---|---|---|
| `FACTORIO_EXE` | the first Steam library that has the game | the game to start |
| `FACTORIO_MODS` | `%APPDATA%\Factorio\mods` | the mods folder the mod manager and start-up report read |
| `FNATIVE_PYPATH` | this repo's `py` folder | folders with Python modules Lua can call, `;`-separated |
| `FNATIVE_PLUGINS` | `dist\plugins` | where plugins are loaded from; empty loads none |
| `FNATIVE_LOG` | `dist\fnative.log` | the log file |
| `FNATIVE_WEB_PORT` | 8790 | the local web API's port (it takes the first free one of 8790-8799) |
| `FNATIVE_PANEL=0` | | open pages in the browser instead of the panel over the game |
| `FNATIVE_OVERLAY=0` | | no hub buttons over the main menu (to hide just one, use the dashboard's *Main menu buttons*) |
| `FNATIVE_STD_WHEEL=0` | | don't watch the mouse wheel (the `std` plugin's wheel events) |
| `FNATIVE_FIXES=0`, `FNATIVE_FIX_<NAME>=0` | | turn off all engine fixes, or one |
| `FNATIVE_DIAG=1` | | log every engine error the game raises, even ones it catches itself |

## Using it from a mod

`native` exists in every Lua state (settings, data and control stage of every mod) when the game was started
through the launcher. Always check for it, so your mod still works without it:

```lua
if native then
  native.log("hello from " .. script.mod_name)
  local out, err = native.call("std", "clipboard_get")
end
```

| function | does |
|---|---|
| `native.version()` | the core's version |
| `native.log(text)` | a line in fnative.log |
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

Put a module on `FNATIVE_PYPATH` with functions that take a string and return a string:

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
`native.call("py", "_reload", "mytool")` reloads a module while the game runs. `print` goes to fnative.log.
Python exceptions come back as `nil, traceback`.

### The `fnative-std` library

```lua
local window = require("__fnative-std__/window")
local events = require("__fnative-std__/events")
local safe   = require("__fnative-std__/safe")
```

| module | gives |
|---|---|
| `window` | `create(player, {name, title, width, height, resizable, close, min_width, min_height})` → frame, content: a title bar to move it, a close button, a corner grip to resize; position and size remembered per player; `on_close`, `on_resize`, `size` |
| `dnd` | `draggable(el, payload)`, `droppable(el, accept)`, `on_drop(fn)`: press, drag (a ghost follows the cursor, the target lights up), release |
| `input` | the cursor and mouse buttons (`read()`), the wheel, clipboard, real time in ms, reading files under script-output |
| `events` | `events.register({...})`: one registration per event for all of the above plus your own handlers, each guarded |
| `safe` | `guard(name, fn)` (an error is logged instead of crashing the game), `has(plugin)`, `log`, `chain` |

Add `"? fnative-std"` to your mod's dependencies if it should work without it, and check
`script.active_mods["fnative-std"]` before requiring.

## Plugins

| plugin | gives |
|---|---|
| `py` | Python functions from Lua (above) |
| `std` | mouse position and buttons in GUI pixels, the wheel (with a lease so the game camera doesn't zoom while your GUI uses it), modifier keys, window size and focus, clipboard, real time, opening http(s) links |
| `web` | a local HTTP API and pages on `127.0.0.1` (token in `dist\web-token.txt`): status, the profiler, native calls, `remote.call` through `fnative-bridge`; `web.open(page)` shows a page in a panel over the game |
| `profiler` | times engine functions by name (`dist\plugins\profiler.json`): Game::update, entity updates by kind, belts, electric networks, robots, pathfinder, Lua events. Only between `profiler.start` and `profiler.stop` |
| `fixes` | small engine bug fixes, each checking the exact bytes it expects first and skipping itself (logged) if a game update changed them |
| `diag` | off unless `FNATIVE_DIAG=1`: logs the message and stack of every engine error |
| `hello` | the C example |

### Engine fixes

- **data-cache-tilde**: with `[other] cache-prototype-data=true` in config.ini, Factorio can skip every mod's data
  stage at start. The cache never loads if any enabled mod has a `~` dependency: the cache file doesn't store that
  flag but the check compares it. The fix leaves that flag out of the check. On a 500-mod pack, starts went from 90 s
  to 50 s. Details in [docs/bug-report-data-cache.md](docs/bug-report-data-cache.md).
- **gc-idle-skip**: every tick the engine forces a Lua GC step in each mod's Lua state, even states that allocated
  nothing since their last cycle, so they re-walk their whole heap again and again. The fix skips the step only for
  a state that is between cycles and whose memory hasn't changed. With ~400 mods this halved the GC time per tick.

## How it works

```
factorio-native.exe   starts factorio.exe suspended, injects fnative.dll, waits for it to be ready, resumes the game
fnative.dll (Rust)    reads the function symbols of factorio.pdb (~100k functions, 60 ms)
                      finds the game's Lua 5.2 C API by name and hooks luaopen_base:
                      every Lua state gets a global `native` table
                      loads the plugins (include/fnative.h) from the plugins folder
```

The launcher keeps the game in a job object, so closing the launcher ends the game too.

**Game updates.** At every start, before anything is hooked:

1. factorio.exe's build id must match factorio.pdb's. If they differ (a half-applied update), nothing is hooked, the
   game runs plain and the log says why.
2. Functions are read fresh from the pdb, so new addresses need nothing done.
3. What our code relies on (`needs\*.needs.json`, `plugins\<plugin>.needs.json`: functions, classes and fields) is
   checked. A missing core function stops the core; a plugin missing something is the only one turned off.
4. On the first start of a new build, `dist\cache\<build>\report.txt` lists what moved since the previous build.

Plugins never hard-code engine offsets: they ask the host (`field_offset("Inserter", "heldStack")`). `fnative-pdb`
(in `target\release`) explores a build from the command line: `info`, `functions Inserter::`, `classes Transport`,
`class Inserter`, `report`.

## Writing a plugin

A plugin is a DLL exporting `fnative_plugin_init(host)` that registers functions taking and returning strings. See
[include/fnative.h](include/fnative.h) and [plugins/hello-c/hello.c](plugins/hello-c/hello.c) (40 lines). Rust
plugins share `crates/fnative-plugin` (the host table and an `export!` macro); `crates/fnative-std` is a complete
example. Put the DLL in `dist\plugins`.

## Tests

The scripts under `test\` start a headless or real game with their own write-data folder, so they never touch your
saves or mods. They find the game like the launcher does (`FACTORIO_EXE` to override).

| script | checks |
|---|---|
| `run_demo.py` | `native` in every stage, plugins, errors, worker jobs, call cost, Python |
| `run_build_check.py` | the build report, a known build, a faked update's diff, a mismatched pdb refused |
| `run_fixes.py` | the data cache fix with a `~` dependency mod |
| `run_web.py` | the web API, bridge and agents end to end |
| `run_std_gui.py` | `fnative-std` drag & drop and resizing in a real game window, with screenshots |
| `run_hub.py` | the menu overlay, the hub and its tabs in a real game window |

## Related

- [bpgen](https://github.com/watdafawx/bpgen): a production-line blueprint planner that runs inside the game through
  fnative and previews the blueprint with the game's own renderer.
- Prior art: Rivets (Rust, DLL injection and pdb symbols, Factorio 1.1).

## License

MIT, see [LICENSE](LICENSE).
