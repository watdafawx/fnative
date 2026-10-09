-- fse-std's own script: multiplayer without desyncs (see the core's mp.rs).
--  * "fse-event" {events = {{plugin, name, data}...}}: what engine hooks saw during this tick's update, raised at its
--    end, the same on every peer
--  * "fse-sync" {player_index, key, data}: what a player's peer sent with native.sync(key, data) (not "name": the
--    game sets every event's name to its id)
--  * the handshake: a player who joins a multiplayer game must run the same fse as the game (version and plugins),
--    or is kicked: one peer without it would desync everyone's mods that use it.
-- Mods listen with script.on_event("fse-event", fn) / script.on_event("fse-sync", fn).

local HELLO_TIMEOUT = 600 -- ticks

-- (log() writes the peer's own log file only: no game state)
local function kick(index, reason)
  log("FSE: kicking player " .. index .. ": " .. reason)
  game.kick_player(index, reason)
end

local function signature()
  if not native then return nil end
  local names = {}
  for p in pairs(native.plugins()) do names[#names + 1] = p end
  table.sort(names)
  return native.version() .. " " .. table.concat(names, ",")
end

local function setup()
  if native and native.on_tick_end then
    -- (a global the core calls at the end of each tick that had simulation events)
    __fse_tick_end = function(events) script.raise_event("fse-event", { events = events }) end
    native.on_tick_end()
  end
end

local function remember()
  storage.signature = signature()
  storage.pending = storage.pending or {}
end

script.on_init(function() setup(); remember() end)
script.on_configuration_changed(remember)
script.on_load(setup)

-- "/fse-sync <name> <data>": the input action native.sync sends; it runs on every peer
commands.add_command("fse-sync", "used by the FSE loader (native.sync); not for typing", function(c)
  local name, data = (c.parameter or ""):match("^(%S+) ?(.*)$")
  if not name or not c.player_index then return end
  if name == "fse-hello" then
    storage.pending[c.player_index] = nil
    if data ~= storage.signature then
      kick(c.player_index, "this game runs FSE " .. tostring(storage.signature) .. "; you have " .. data)
    end
    return
  end
  script.raise_event("fse-sync", { player_index = c.player_index, key = name, data = data })
end)

script.on_event(defines.events.on_player_joined_game, function(e)
  if not game.is_multiplayer() or not storage.signature then return end -- (a game without fse: nothing to compare)
  storage.pending[e.player_index] = e.tick
  -- (only the joining player's own peer sends; sending changes nothing in the game)
  if native and native.local_player and native.local_player() == e.player_index then
    native.sync("fse-hello", signature())
  end
end)

script.on_event(defines.events.on_player_left_game, function(e)
  storage.pending[e.player_index] = nil
end)

script.on_nth_tick(60, function(e)
  for index, since in pairs(storage.pending) do
    if e.tick - since > HELLO_TIMEOUT then
      storage.pending[index] = nil
      kick(index, "this game needs the FSE loader " .. tostring(storage.signature) .. " (github.com/watdafawx/fse)")
    end
  end
end)
