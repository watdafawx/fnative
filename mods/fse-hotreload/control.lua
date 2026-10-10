-- fse-hotreload: edits to your mods' source folders reach the running game (singleplayer only; fse's py plugin).
-- Every half second fse_tools.hotreload compares the folders listed in FSE_HOTRELOAD (fse.env) with what it saw last;
-- changed .lua files are compiled here first (a syntax error in a reloaded control.lua would end the game), then the
-- source is copied over the installed mod and every control.lua reloads (game.reload_script: storage kept, on_load runs).
-- Changes to anything else (data.lua, settings, locale, graphics) only load at the game's start: the game is saved
-- (_autosave-hotreload) and started again on that save.

if not native then return end

local broken = false

local function py(fn, input)
  local out, err = native.call("py", "fse_tools.hotreload:" .. fn, input)
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

script.on_nth_tick(30, function()
  if broken or game.is_multiplayer() then return end
  local names = {}
  for name in pairs(script.active_mods) do names[#names + 1] = name end
  local r = py("check", helpers.table_to_json({ mods = names }))
  if not r then return end
  for _, note in pairs(r.notes) do say(note) end
  local ok, restart = {}, {}
  for _, m in pairs(r.changed) do
    local bad
    for path, source in pairs(m.lua) do
      local _, err = load(source, "=__" .. m.name .. "__/" .. path, "t", {})
      if err then bad = err break end
    end
    if bad then
      say(m.name .. " not reloaded: " .. bad, { 1, 0.45, 0.4 })
    else
      ok[#ok + 1] = m.name
      if #m.restart > 0 then restart[#restart + 1] = m.name .. " (" .. table.concat(m.restart, ", ") .. ")" end
    end
  end
  if #ok == 0 then return end
  py("apply", helpers.table_to_json(ok))
  if #restart > 0 then
    local err = native.call("py", "fse_tools.hotreload:restart", helpers.table_to_json({ save = "hotreload" }))
    if err == "" then
      say("restarting on this save for " .. table.concat(restart, "; "), { 1, 0.8, 0.3 })
      game.auto_save("hotreload")
      return
    end
    say("can't restart (" .. tostring(err) .. "): " .. table.concat(restart, "; ") .. " need one", { 1, 0.45, 0.4 })
  end
  say(table.concat(ok, ", ") .. " reloaded", { 0.5, 1, 0.5 })
  game.reload_script()
end)
