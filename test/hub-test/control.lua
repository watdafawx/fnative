-- drives the fnative hub in a real client: the Hub tab with the profiler, the Mods and Startup tabs once their data
-- is in (screenshots), then a web page in the panel over the game
local mod_gui = require("mod-gui")
local step, waited = "hub", 0

local function shot(name)
  local p = game.get_player(1)
  game.take_screenshot({ player = 1, show_gui = true, path = name,
    resolution = { p.display_resolution.width, p.display_resolution.height }, zoom = 1 })
end

local function log(text) helpers.write_file("hub-result.txt", text .. "\n", true) end

script.on_event(defines.events.on_tick, function(e)
  local p = game.get_player(1)
  if e.tick == 60 then
    helpers.write_file("hub-result.txt", "", false)
    remote.call("fnative-hub", "open", 1, "hub")
    if native then native.call("profiler", "start") end
  elseif e.tick == 260 then
    log("mod-gui button: " .. tostring(mod_gui.get_button_flow(p).fnative_hub_button ~= nil))
    log("hub open: " .. tostring(p.gui.screen.fnative_hub ~= nil))
    shot("hub-ingame.png")
  elseif e.tick > 300 and (step == "hub" or step == "mods" or step == "startup") then
    if step == "hub" then step = "mods"; waited = 0; remote.call("fnative-hub", "open", 1, "mods") return end
    waited = waited + 1
    local got = remote.call("fnative-hub", "loaded", step)
    if got or waited > 1800 then
      log(step .. " data: " .. tostring(got) .. " after " .. waited .. " ticks")
      step = step .. "-shot"; waited = 0
    end
  elseif step == "mods-shot" or step == "startup-shot" then
    waited = waited + 1
    local tab = step:sub(1, -6)
    if waited == 30 then
      shot("hub-" .. tab .. ".png")  -- (taken at the end of this tick: switch tabs on a later one)
    elseif waited == 32 then
      if tab == "mods" then
        step = "startup"; waited = 0; remote.call("fnative-hub", "open", 1, "startup")
      else
        step = "panel"; waited = 0
        log("web.open: " .. tostring(native.call("web", "open", "mods.html")))
      end
    end
  elseif step == "panel" then
    waited = waited + 1
    if waited == 600 then helpers.write_file("hub-done.txt", "1", false) end
  end
end)
