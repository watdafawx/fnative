-- multiplayer test: everything here changes the game state from fse's two ways in, so a peer that saw them
-- differently desyncs. The server rotates an inserter every 30 ticks (a hooked engine call: "fse-event" counts it in
-- storage); each player's own peer sends local data (its real clock) with native.sync ("fse-sync" stores it). Each
-- peer writes what it saw to script-output/mp-<peer>.txt.
local ROTATE = { fn = "?rotate@Entity@@UEAA?AVActionResult@@W4RotateDirection@@@Z", event = "mp-rotate",
                 args = { { arg = 0, path = "prototype.name", as = "name" }, { arg = 0, path = "position", as = "pos" } } }

local function hook()
  if native then
    native.call("hooks", "add", helpers.table_to_json(ROTATE))
  end
end

script.on_init(function()
  local fp = remote.interfaces["freeplay"]
  if fp then
    if fp.set_skip_intro then remote.call("freeplay", "set_skip_intro", true) end
    if fp.set_disable_crashsite then remote.call("freeplay", "set_disable_crashsite", true) end
  end
  storage.rotations, storage.synced, storage.positions = 0, {}, {}
  storage.inserter = game.surfaces[1].create_entity({ name = "inserter", position = { 0.5, 0.5 }, force = "player" })
  hook()
end)
-- (hooks aren't in the save: a joining peer adds them as it loads the map)
script.on_load(hook)

script.on_event("fse-event", function(e)
  for _, ev in ipairs(e.events) do
    if ev.name == "mp-rotate" then
      storage.rotations = storage.rotations + 1
      storage.positions[#storage.positions + 1] = ev.data.args.pos.x
    end
  end
end)

script.on_event("fse-sync", function(e)
  if e.key == "mp-clock" then
    storage.synced[#storage.synced + 1] = { player = e.player_index, data = e.data, tick = game.tick }
  end
end)

script.on_nth_tick(30, function(e)
  if storage.inserter and storage.inserter.valid then storage.inserter.rotate() end
  -- each player's own peer sends its local clock (only that peer knows it) a few times
  for _, p in pairs(game.connected_players) do
    if native and native.local_player() == p.index and e.tick % 120 == 0 then
      local now = native.call("std", "now") or "?"
      native.sync("mp-clock", now)
    end
  end
  if e.tick % 300 == 0 then
    local text = ("tick %d rotations %d synced %d players %d"):format(e.tick, storage.rotations, #storage.synced,
      #game.connected_players)
    -- (every peer writes its own file: the server as peer 0, each player's peer under its player index)
    helpers.write_file("mp-server.txt", text .. "\n", true, 0)
    for _, p in pairs(game.connected_players) do
      helpers.write_file("mp-player-" .. p.index .. ".txt", text .. "\n", true, p.index)
    end
  end
end)

