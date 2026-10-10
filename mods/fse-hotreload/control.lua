-- fse-hotreload: edits to your mods' source folders reach the running game (singleplayer only; fse's py plugin).
-- Every half second fse_tools.hotreload compares the folders listed in FSE_HOTRELOAD (fse.env) with what it saw last.
-- Control-stage changes: compiled here first (a syntax error in a reloaded control.lua would end the game), then the
-- source is copied over the installed mod and every control.lua reloads (game.reload_script: storage kept, on_load runs).
-- Changes to anything else (data.lua, settings, locale, graphics) only load at the game's start, and come in several
-- saves: the mod is held (not copied, not reloaded) and a "Restart for <mod>" button waits in the top left. Pressing it
-- (or remote.call("fse-hotreload", "restart"), e.g. over fse's web API) saves the game (_autosave-hotreload) and starts
-- it again on that save.

if not native then return end

local mod_gui = require("mod-gui")
local BUTTON = "fse_hotreload_restart"
local broken = false

local function py(fn, input)
  local out, err = native.call("py", "fse_tools.hotreload:" .. fn, input or "")
  if not out then
    broken = true  -- (logged once, not every half second)
    log("fse-hotreload: " .. tostring(err))
    return nil
  end
  return helpers.json_to_table(out)
end

local function say(text, color)
  log("fse-hotreload: " .. text)
  game.print("[hot reload] " .. text, { color = color, skip = defines.print_skip.never })
end

local RED, GREEN, AMBER = { 1, 0.45, 0.4 }, { 0.5, 1, 0.5 }, { 1, 0.8, 0.3 }

-- the first syntax error in a list of {name, lua = {path = source}}, as "mod: error"
local function syntax_error(mods)
  for _, m in pairs(mods) do
    for path, source in pairs(m.lua) do
      local _, err = load(source, "=__" .. m.name .. "__/" .. path, "t", {})
      if err then return m.name, err end
    end
  end
end

-- the button in every player's mod button flow while mods are held, gone when none are
local function show_held(held)
  local names, lines = {}, {}
  for name, paths in pairs(held) do
    names[#names + 1] = name
    lines[#lines + 1] = name .. ": " .. table.concat(paths, ", ")
  end
  table.sort(names)
  for _, player in pairs(game.connected_players) do
    local flow = mod_gui.get_button_flow(player)
    local button = flow[BUTTON]
    if #names == 0 then
      if button then button.destroy() end
    else
      button = button or flow.add({ type = "button", name = BUTTON, style = "red_button" })
      button.caption = "Restart for " .. table.concat(names, ", ")
      button.tooltip = "Changed, loads only at the game's start:\n" .. table.concat(lines, "\n") ..
        "\n\nPress when your edits are done: the game is saved and started again on that save."
    end
  end
end

local function restart()
  local held = py("held")
  if not held then return "fse's py plugin isn't working" end
  if #held == 0 then return "nothing waits for a restart" end
  local name, err = syntax_error(held)
  if err then
    say(name .. " not restarted: " .. err, RED)
    return err
  end
  local names = {}
  for _, m in pairs(held) do names[#names + 1] = m.name end
  local why = native.call("py", "fse_tools.hotreload:restart", helpers.table_to_json({ save = "hotreload" }))
  if why ~= "" then
    say("can't restart: " .. tostring(why), RED)
    return why
  end
  py("apply", helpers.table_to_json(names))
  say("restarting on this save for " .. table.concat(names, ", "), AMBER)
  game.auto_save("hotreload")
  return "restarting"
end

remote.add_interface("fse-hotreload", { restart = restart })

script.on_event(defines.events.on_gui_click, function(e)
  if e.element.valid and e.element.name == BUTTON and not game.is_multiplayer() then restart() end
end)

script.on_nth_tick(30, function()
  if broken or game.is_multiplayer() then return end
  local names = {}
  for name in pairs(script.active_mods) do names[#names + 1] = name end
  local r = py("check", helpers.table_to_json({ mods = names }))
  if not r then return end
  for _, note in pairs(r.notes) do say(note) end
  show_held(r.held)
  local ok = {}
  for _, m in pairs(r.changed) do
    local name, err = syntax_error({ m })
    if err then say(name .. " not reloaded: " .. err, RED) else ok[#ok + 1] = m.name end
  end
  if #ok == 0 then return end
  py("apply", helpers.table_to_json(ok))
  say(table.concat(ok, ", ") .. " reloaded", GREEN)
  game.reload_script()
end)
