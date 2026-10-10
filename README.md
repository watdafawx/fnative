# FSE (Factorio Script Extender)

A native extension layer for Factorio 2.0 on Windows. Native code (Rust, C, anything with a C ABI, and Python) runs
inside the game and Lua mods reach it through a global `native` table. With it, mods can do things the Lua API
can't: read the mouse, use the clipboard, run work on threads, call Python libraries, show web pages over the game,
profile engine functions and patch small engine bugs.

Installing is unzipping into the game's folder; uninstalling is deleting what that added. No launcher, no Steam
launch options: a small `version.dll` beside `factorio.exe` (Windows loads it from the game's folder) loads
`fse.dll` before the game starts. fse finds the engine's functions by name in `factorio.pdb`, the debug symbols Wube
ships with the game, so game updates need no new offsets, and Steam updates leave the install alone.

> **Multiplayer works** when every player and the server run the same FSE (see *Multiplayer* below: a player
> without it is kicked with a message saying so). Windows only. Not for the mod portal (it can't carry native
> binaries). Keep crash-report uploading off while you experiment: crashes with foreign code in the process are
> noise for Wube.

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
| `fse-hub` | an **F** button (top left) with tabs: loader and plugins, your mods (added/updated dates, real load order, portal updates), the start-up time report per mod, the engine profiler, and **Get mods**: the fse mod catalog ([fse-mods](https://github.com/watdafawx/fse-mods)), installed with one click and checked against the index's sha256 (they load at the next start). Needs `fse-std`. |
| `fse-bridge` | runs commands from the local web API on the game thread. |
| `fse-agent` | characters without a player that an external program (an AI agent, `examples/agent_client.py`) can drive. |
| `fse-hotreload` | for mod developers: list your mods' source folders in `fse.env` (`FSE_HOTRELOAD`); saving a file copies the mod over its installed folder and the running game reloads its control.lua within half a second (`game.reload_script`: `storage` kept, `on_load` runs). Changed files are compiled first, so a syntax error shows in chat and the old code keeps running. A data.lua, settings, locale or graphics change restarts the game on a fresh save (`_autosave-hotreload`, about 10 s for a small modlist), because prototypes only load at start. Singleplayer only, needs `python` on PATH for the restart, and the installed mod has to be a folder, not a zip. |

Every mod checks for fse first and does nothing (or falls back) without it.

Settings go in `fse\fse.env` (optional: copy `fse\fse.env.example` to it and see the comments; unzipping a newer
release over the install never replaces it). The log is `fse\fse.log`. A good start:

```
fse 0.10.0 loading
105032 engine symbols read in 61.48ms
game build EAAB184A...-1 (known)
plugin ...\fse_std.dll: init ok
ready in 70.97ms
```

**Updates:** when a newer fse is out, the hub says so (its Hub tab, the dashboard behind **FSE hub** in the menus,
and once in the chat). **Update** downloads it from the GitHub release, checks it against the release's sha256 and
installs it over this one (your `fse.env` stays); the game uses it from its next start.

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

`dist\fse-launcher.exe` is the launcher the tests use: it starts the game and injects `fse.dll` (from beside itself) with
nothing installed. Pass it any Factorio arguments; it finds the game in the usual Steam libraries, or set
`FACTORIO_EXE`.

### 3. Check it works

In game, open the console and run:

```
/c game.print(native and native.version() or "no loader")
```

Or click the **F** button if you installed `fse-hub`. In the game's main menu and pause menu (Esc), **FSE hub** (under Settings)
opens the dashboard.

## Settings (`fse\fse.env`)

One `KEY=value` per line, read at every start (by the loader, or the launcher beside `dist\fse-launcher.exe`).

| key | default | does |
|---|---|---|
| `FACTORIO_EXE` | the first Steam library that has the game | the game the launcher starts |
| `FACTORIO_MODS` | the game's own (`--mod-directory`, else its config) | the mods folder fse's mods go into, and the mod manager and start-up report read |
| `FSE_PYPATH` | | more folders with Python modules Lua can call, `;`-separated (fse's `py` folder is always there) |
| `FSE_PLUGINS` | `fse\plugins` | where plugins are loaded from; empty loads none |
| `FSE_LOG` | `fse\fse.log` | the log file |
| `FSE_WEB_PORT` | 8790 | the local web API's port (it takes the first free one of 8790-8799) |
| `FSE_PANEL=0` | | open pages in the browser instead of the panel over the game |
| `FSE_MENU_BUTTON=0` | | no **FSE hub** button in the game's main and pause menus (then the overlay buttons below show instead) |
| `FSE_OVERLAY=0` | | no hub buttons over the main menu when the menu button can't be added (to hide just one, use the dashboard's *Main menu buttons*) |
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
| `native.read(target, path?, depth?)` | an engine object's field, decoded (below) |
| `native.layout(class)` | an engine class as this build has it: `{name, size, fields = {{name, offset, type}}}` |
| `native.metatable(object)` | a game object's metatable, past its protection (`fse-std`'s `extend` uses it) |
| `native.sync(key, data)` | sends `data` (a string) from this peer's player to every peer: the `fse-sync` event `{player_index, key, data}` (multiplayer, below) |
| `native.local_player()` | this peer's own player index, or nil (headless server, menu) |
| `native.send_action(kind, text?)` | an input action as this peer's player makes it (kinds without data, like `OpenCharacterGui`, or with a text, like `WriteToConsole`; `LuaShortcut` with a shortcut's name clicks another mod's shortcut), through the game's own pipeline: every peer applies it |
| `native.root()` | the game's root objects as pointers for `native.read`: `{scenario, game, map, local_player}` |
| `native.write(target, path, value)` | writes a number or boolean field: for this peer's own state (camera, UI); writing simulation state desyncs multiplayer |
| `native.tick_stats()` | how long the update step took: `{last_ms, avg_ms, max_ms}` (this peer's own) |
| `native.events(since?)` | events plugins sent after `since`: `{ {seq, plugin, name, data} }, newest`; without `since` just `newest` |

Rules: native functions never raise Lua errors; they return `nil, message`. Inputs and outputs are strings (JSON by
convention). Long work goes through `start`/`poll` so a tick never waits.

### Engine objects

`native.read` reads the game's own C++ objects by their field names, from the type records in `factorio.pdb`, so
it keeps working across game updates as long as the names do. A target is a game Lua object (`LuaEntity`,
`LuaPlayer`, `LuaSurface`, ...), or a pointer a read handed out (`{ptr = n, type = "Inserter"}`).

```lua
local ins = native.read(entity, "entityTarget.target")   -- {ptr = ..., type = "Inserter"}: the real class
native.read(ins, "heldStack")                             -- {count = 0, itemID = 0, stackSize = 0, ...}
native.read(ins, "prototype.name")                        -- "inserter": through a pointer and base classes
native.read(entity, "entityTarget.target.position")       -- {x = 10.5, y = 7.5}
native.read(ins, "^Entity.surface")                       -- ^Class views the object as a base or derived class
```

Paths are field names joined by dots; fields of base classes are found too, pointers are followed, and a number
indexes an array or `std::vector`. Values come back as numbers, booleans, strings (`std::string`), tables of
fields (`depth` levels deep, default 1) and pointers. Pointers to classes resolve to the object's real class via
RTTI. A bad address or unknown field returns `nil, message`; nothing is ever read unchecked. Explore with
`native.layout("Inserter")` or `fse-pdb layout Inserter` (`target\release\fse-pdb.exe`). Reading is for looking:
nothing is written.

### Multiplayer

Factorio's multiplayer is lockstep: every peer simulates the same ticks from the same input actions. Anything only
one peer knows (the mouse, the clipboard, a file, Python's answer, the clock, an engine address) must reach the game
through those input actions, or the peers drift apart (a desync). FSE gives two ways in, both arriving on every peer
in the same tick (`fse-std` raises them; it must be installed):

```lua
-- 1. engine hooks: calls the simulation made during the tick's update, at its end, in a fixed order
script.on_event("fse-event", function(e)
  for _, ev in ipairs(e.events) do  -- {plugin, name, data}
    if ev.name == "rotated" then storage.rotations = storage.rotations + 1 end
  end
end)

-- 2. local data: one peer sends it, every peer gets it
if native.local_player() == player.index then native.sync("my-mod:clock", native.call("std", "now")) end
script.on_event("fse-sync", function(e)  -- {player_index, key, data}
  if e.key == "my-mod:clock" then storage.clocks[e.player_index] = e.data end
end)
```

The rules:

- Change the game only from deterministic code (events, `on_tick`, commands) using game state, `fse-event` and
  `fse-sync`. `native.events`, `native.call` results, `native.local_player()` and the pointers `native.read` hands
  out are this peer's own: use them for what this peer shows or sends, never to change the game directly.
- What plugins keep isn't in the save: add hooks (`hooks.add`) and blocked inputs (`input.block`) in both `on_init`
  and `on_load`, so a joining player has them before its first tick. Values `native.read` reads from simulation
  objects are the same on every peer.
- Hooks on functions the render thread runs need `local = true` (they never join `fse-event`).
- `native.sync` needs a player: a headless server can't send. Keep the data small; it travels as a console command.
- In multiplayer `fse-std`'s windows and drag & drop work as they do without FSE (click to pick and drop), since
  the mouse is one peer's own.
- The handshake: every joining player's FSE sends its version and plugin list; a different one, or none within 10
  seconds, is kicked.

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
| `native_events` | `on(plugin, event, fn)`: events from plugins (engine hooks) to handlers; add `handlers` to `events.register` (it polls each tick) |
| `extend` | `class(sample, {name = fn})`: new properties (and `extend.method`s) on a game class like `LuaEntity`, for your mod only |

Add `"? fse-std"` to your mod's dependencies if it should work without it, and check
`script.active_mods["fse-std"]` before requiring.

## Plugins

| plugin | gives |
|---|---|
| `py` | Python functions from Lua (above) |
| `std` | `play_sound("__mod__/x.wav")` (a .wav from an unzipped mod, or under script-output, on this computer only), mouse position and buttons in GUI pixels, the wheel (with a lease so the game camera doesn't zoom while your GUI uses it), modifier keys and every key held, window size and focus, clipboard, real time, opening http(s) links; `press {scancode \| vk \| mouse, mods, phase, hold_ms}` posts a key or mouse button to the game window as if pressed here (the game's controls and every mod's custom inputs bound to it fire), `key_info(vk)` names a key |
| `web` | a local HTTP API and pages on `127.0.0.1` (token in `fse\web-token.txt`): status, the profiler, native calls, `remote.call` through `fse-bridge`; `web.open(page)` shows a page in a panel over the game; adds **FSE hub** to the game's main and pause menus (hooks the menus' `addResultButton`; a build without them gets the overlay buttons) |
| `profiler` | times engine functions by name (`fse\plugins\profiler.json`): Game::update, entity updates by kind, belts, electric networks, robots, pathfinder, Lua events. Only between `profiler.start` and `profiler.stop` |
| `fixes` | small engine bug fixes, each checking the exact bytes it expects first and skipping itself (logged) if a game update changed them |
| `diag` | off unless `FSE_DIAG=1`: logs the message and stack of every engine error |
| `entityinfo` | a mod's own rows in the game's info panel for the entity under the cursor (below) |
| `draw` | lines, rectangles, circles and text drawn over the game world every frame, on this computer only (safe in multiplayer): `draw.set {id, surface, shapes}`, `draw.clear {id}` (below) |
| `hooks` | any engine function, by its pdb name, as an event for Lua on every call, with chosen arguments read (below) |
| `input` | every player input action (336 kinds: `native.layout("InputActionType").values`) as an event `action` `{type, player, tick, blocked}` (`input.watch {types}` or `{all = true}`), and chosen kinds dropped before the game applies them (`input.block {types, player?}`); `input.controls`: every control the game has (its own and each mod's custom inputs) with the keys bound to it, as Settings > Controls shows them; `input.trigger {control}` presses a control's key into the game window |
| `hello` | the C example |

### Engine fixes

- **data-cache-tilde**: with `[other] cache-prototype-data=true` in config.ini, Factorio can skip every mod's data
  stage at start. The cache never loads if any enabled mod has a `~` dependency: the cache file doesn't store that
  flag but the check compares it. The fix leaves that flag out of the check. On a 500-mod pack, starts went from 90 s
  to 50 s. Details in [docs/bug-report-data-cache.md](docs/bug-report-data-cache.md).
- **gc-idle-skip**: every tick the engine forces a Lua GC step in each mod's Lua state, even states that allocated
  nothing since their last cycle, so they re-walk their whole heap again and again. The fix skips the step only for
  a state that is between cycles and whose memory hasn't changed. With ~400 mods this halved the GC time per tick.

### Drawing over the world (`draw`)

```lua
native.call("draw", "set", helpers.table_to_json({ id = "my-mod:route", surface = player.surface.index, shapes = {
  { line = { { 0, 0 }, { 10, 5 } }, color = { 0.3, 0.6, 1 }, width = 2 },
  { rect = { { 9.5, 4.5 }, { 10.5, 5.5 } }, color = { 1, 0.2, 0.2 }, width = 3 },    -- width 0: filled
  { circle = { 0, 0 }, radius = 2, color = { 0.2, 1, 0.2, 0.8 } },
  { text = "the chest", at = { 9.5, 4.2 }, size = 18 },
} }))
native.call("draw", "clear", helpers.table_to_json({ id = "my-mod:route" }))
```

Positions in tiles, widths and text size in screen pixels, colours 0..1. It is this computer's own (no game state:
safe in multiplayer, and for what only this player should see). Nothing in the game's renderer is touched: a
transparent window that lets clicks through lies over the game's client area, and each time the camera moves the
shapes are drawn where it puts them. They sit above the game's own GUI; not over an exclusive fullscreen game.

### Engine functions as events (`hooks`)

Hook a function by its `factorio.pdb` name (`fse-pdb functions rotate` lists them) and every call becomes an event,
with the arguments you ask for read like `native.read` does:

```lua
local nev = require("__fse-std__/native_events")
native.call("hooks", "add", helpers.table_to_json({
  fn = "?rotate@Entity@@UEAA?AVActionResult@@W4RotateDirection@@@Z", event = "rotated",
  args = { { arg = 0, path = "prototype.name", as = "name" }, { arg = 0, path = "position", as = "pos" } },
}))
nev.on("hooks", "rotated", function(data) game.print(data.args.name .. " rotated") end)
```

A hook: `fn`, `event`, `args` (`arg`: which argument, 0 is `this`; `path`, `class`, `depth`, `as`; or `raw = true`
for the integer itself), `every` (send every Nth call), `after` (read the arguments after the call), `result` (add
the return value). `hooks.enable` / `hooks.disable` `{event}` switch one; `hooks.status` lists them with call counts.
Hooks can also be listed in `fse\plugins\hooks.json` (`{"hooks": [...]}`), installed before the game starts.

Presets, tested ones ready to use: `{preset = "console"}` (every console message, `{message = LocalisedString,
from}`: the game's own lines too, which Lua never sees), `"expansion"` (where biters pick their next base:
`{position, from}`), `"save"` (local: the folder and name a save goes to), `"app-state"` (local: the game's screen
stack after a change: main menu, loading, in game). Other fields override a preset's.
Only functions whose arguments are all integers or pointers (at most 8) can be hooked; prefer functions of the
update and Lua thread. Events reach Lua at the next poll, never inside engine code.

### Rows in the game's entity info panel (`entityinfo`)

Also for prototypes: `entityinfo.set_proto {type = "item", name = "iron-plate", rows = {{"Made here", "120/min"}}}`
(`type`: item, recipe, technology, fluid, entity, or any) adds rows wherever the game describes that prototype:
tooltips in inventories and the crafting menu, Factoriopedia. `clear_proto {type, name}` (or nothing: all).

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
(fse-launcher.exe  the launcher: starts factorio.exe suspended, injects fse.dll, waits, resumes the game)
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
| `run_menu.py` | the **FSE hub** button in the main and pause menus: screenshots, clicks (posted to the game window), the panel opens |
| `run_hub.py` | the menu overlay, the hub and its tabs in a real game window (an install from a local mod index too) |
| `run_catalog.py` | the mod catalog's Python half (`fse_tools.catalog`), without the game |
| `run_update.py` | fse updates (`fse_tools.update`) against a fake release and game folder: sha256, a loaded DLL swapped, `fse.env` kept |
| `run_loader.py` | the installed loader: a direct start loads fse and puts its mods in place; `FSE_OFF`; loader and launcher together |
| `run_input_gui.py` | the `input` plugin in a real game window: a key press as an action event, then blocked |
| `run_controls.py` | `input.controls` (bindings and modifiers of the game's and a mod's controls), then in a real game window `input.trigger`, `std.press` (keys with each modifier, a mouse button) and `send_action("LuaShortcut")` fire what they should |
| `run_mp.py` | a headless server and a client on this machine: simulation events and `native.sync` the same on both, no desync; a client without FSE kicked |
| `run_draw_gui.py` | the `draw` plugin in a real game window, the screen grabbed (`test\run\script-output\draw-screen.png`) |
| `run_engine_api.py` | `native.read`, `layout`, `metatable`, `events`, the `hooks` plugin and `fse-std`'s `extend` and `native_events` |
| `run_hotreload.py` | `fse-hotreload` in a real game window: a probe mod's control.lua edited and reloaded with storage kept; a syntax error refused, the game lives on; a data.lua edit restarts the game on its save with the new prototype |
| `run_mcp_game.py` | `mcp_game.py` over stdio: a probe mod started, Lua in its own state, ticks, an edit picked up by reload with storage kept |

`factorio_paths.py` is shared with the mods' tests (an identical copy in each `test\`): `headless()` runs one test case,
`main()` is a mod's whole `test\run.py` (`--all [-j N]` runs every case, N games at once, at below normal priority).

`mcp_game.py` is an MCP server (stdio, no dependencies) for an agent testing a mod: a warm headless server (its own
`test\run\mcp-game`) driven over the game's RCON. Tools: `start` (a mod folder, optionally a test case, `fse` for the
launcher), `lua` (a function body in a mod's own Lua state, the return value back), `ticks`, `reload` (saved, the mods
copied again, the save loaded: about 3 s, state kept, control.lua and data.lua changes), `log`, `output`, `stop`.
Register it with `{"mcpServers": {"factorio": {"command": "python", "args": ["<path>/fse/test/mcp_game.py"]}}}`.

## Related

- [bpgen](https://github.com/watdafawx/bpgen): a production-line blueprint planner that runs inside the game through
  FSE and previews the blueprint with the game's own renderer.
- Prior art: Rivets (Rust, DLL injection and pdb symbols, Factorio 1.1).

## License

MIT, see [LICENSE](LICENSE).
