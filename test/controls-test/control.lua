-- 1: the control list (input.controls) to script-output/controls-test.json
-- then, in a real game: input.trigger, std.press and native.send_action("LuaShortcut") each fire what they should;
-- the results go to controls-live.txt
local seen = {}
script.on_init(function()
  -- (the crash-site cutscene swallows input while it plays)
  if remote.interfaces.freeplay then
    remote.call("freeplay", "set_skip_intro", true)
    remote.call("freeplay", "set_disable_crashsite", true)
  end
end)
script.on_event("controls-test-combo", function(e) seen[#seen + 1] = "combo@" .. e.tick end)
for _, m in pairs({"ctrl", "shift", "alt"}) do
  script.on_event("controls-test-" .. m, function(e) seen[#seen + 1] = m .. "@" .. e.tick end)
end
script.on_event("controls-test-mouse", function(e) seen[#seen + 1] = "mouse@" .. e.tick end)
script.on_event(defines.events.on_lua_shortcut, function(e)
  seen[#seen + 1] = e.prototype_name .. "@" .. e.tick .. ":p" .. e.player_index
end)

local function call(plugin, fn, t)
  local out, err = native.call(plugin, fn, helpers.table_to_json(t))
  if not out then seen[#seen + 1] = fn .. " error: " .. tostring(err) end
end

script.on_event(defines.events.on_tick, function(e)
  if e.tick == 1 then
    local out, err = native.call("input", "controls", "")
    helpers.write_file("controls-test.json", out or ("error: " .. tostring(err)))
  elseif e.tick == 120 then
    call("input", "trigger", {control = "controls-test-combo"})
  elseif e.tick == 180 then
    call("std", "press", {scancode = "K", mods = {"shift", "alt"}})
  elseif e.tick == 240 then
    call("std", "press", {mouse = "button-4"})
  elseif e.tick == 300 then
    local ok, err = native.send_action("LuaShortcut", "controls-test-sc")
    if not ok then seen[#seen + 1] = "send_action error: " .. tostring(err) end
  elseif e.tick == 330 then
    call("std", "press", {scancode = "J", mods = {"ctrl"}})
  elseif e.tick == 340 then
    call("std", "press", {scancode = "J", mods = {"shift"}})
  elseif e.tick == 350 then
    call("std", "press", {scancode = "J", mods = {"alt"}})
  elseif e.tick == 360 then
    call("input", "trigger", {control = "toggle-map"})
  elseif e.tick == 420 then
    seen[#seen + 1] = "render_mode " .. tostring(game.get_player(1).render_mode)
    helpers.write_file("controls-live.txt", table.concat(seen, "\n"))
  end
end)
