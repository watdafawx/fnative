-- fse-bridge: runs commands queued by the fse web plugin (POST /api/game) on the game thread and answers.
--
-- A command is {interface, function, args}: remote.call(interface, function, table.unpack(args)), so every mod with
-- a remote interface can be driven over HTTP. Built in, under interface "$game":
--   status       tick, players, surfaces, active mod count
--   interfaces   every remote interface and its functions (what an agent can call)
--   print(text)  a chat message
-- Without the fse loader (no `native` table) it does nothing.

local function builtin(fn, args)
  if fn == "status" then
    local players = {}
    for _, p in pairs(game.connected_players) do
      players[#players + 1] = { name = p.name, index = p.index, surface = p.surface.name,
        position = p.position, controller = p.controller_type }
    end
    local surfaces = {}
    for _, s in pairs(game.surfaces) do surfaces[#surfaces + 1] = s.name end
    local mods = 0
    for _ in pairs(script.active_mods) do mods = mods + 1 end
    return { tick = game.tick, ticks_played = game.ticks_played, speed = game.speed, players = players,
      surfaces = surfaces, mods = mods }
  elseif fn == "interfaces" then
    local out = {}
    for name, fns in pairs(remote.interfaces) do
      local list = {}
      for f in pairs(fns) do list[#list + 1] = f end
      table.sort(list)
      out[name] = list
    end
    return out
  elseif fn == "print" then
    game.print("[fse] " .. tostring(args[1]))
    return true
  end
  error("no built-in " .. tostring(fn) .. " (status, interfaces, print)")
end

local function run(c)
  local args = c.args or {}
  if c.interface == "$game" then return pcall(builtin, c["function"], args) end
  local iface = remote.interfaces[c.interface]
  if not iface then return false, "no remote interface " .. tostring(c.interface) end
  if not iface[c["function"]] then return false, c.interface .. " has no function " .. tostring(c["function"]) end
  return pcall(remote.call, c.interface, c["function"], table.unpack(args))
end

local function answer(c, ok, res)
  local reply = { id = c.id, ok = ok }
  if ok then reply.result = res else reply.error = tostring(res) end
  local okj, text = pcall(helpers.table_to_json, reply)
  if not okj then  -- (a Lua object in the result can't be JSON: say so instead)
    text = helpers.table_to_json({ id = c.id, ok = false, error = "result can't be sent as JSON: " .. tostring(text) })
  end
  native.call("web", "reply", text)
end

-- (guarded: a bad command or answer is logged and skipped, never the end of the game)
local function guarded(fn)
  return function(e)
    local ok, err = pcall(fn, e)
    if not ok then log("[" .. script.mod_name .. "] error (handled): " .. tostring(err)) end
  end
end
script.on_event(defines.events.on_tick, guarded(function()
  -- (not in the main menu's background simulation: it runs mods too, and the hub button over the menu must stay)
  if not native or script.level.is_simulation then return end
  local cmds = native.call("web", "take")
  if not cmds or cmds == "[]" then return end
  for _, c in pairs(helpers.json_to_table(cmds) or {}) do
    answer(c, run(c))
  end
end))
