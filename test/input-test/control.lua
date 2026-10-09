-- the input plugin in a real client: the test script presses E (open the inventory) twice through the window's
-- messages; the first press must arrive as an action event, the second, with that kind blocked, must not open it
local nev = require("__fse-std__/native_events")
script.on_init(function()
  local fp = remote.interfaces["freeplay"]
  if fp then
    if fp.set_skip_intro then remote.call("freeplay", "set_skip_intro", true) end
    if fp.set_disable_crashsite then remote.call("freeplay", "set_disable_crashsite", true) end
  end
end)

local out, seen, phase, kind, phase_tick = {}, {}, 0, nil, 0
local function say(s) out[#out + 1] = s end
nev.on("input", "action", function(d) seen[#seen + 1] = d end)
sim_seen, local_seen = {}, {}
nev.on("hooks", "save", function(d) local_seen.save = local_seen.save or d end)
script.on_event("fse-event", function(e)
  for _, ev in ipairs(e.events) do
    if ev.plugin == "hooks" and (ev.name == "console" or ev.name == "expansion") then
      sim_seen[ev.name] = sim_seen[ev.name] or ev.data
    end
  end
end)

local function finish()
  helpers.write_file("input-result.txt", table.concat(out, "\n") .. "\n", false)
  helpers.write_file("input-done.txt", "1", false)
end

script.on_event(defines.events.on_tick, function(e)
  nev.poll()
  if not native then if e.tick == 30 then say("FAIL no native"); finish() end return end
  local p = game.get_player(1)
  if phase == 0 and e.tick == 60 then
    native.call("input", "watch", '{"all": true}')
    phase = 1
    seen = {}
    helpers.write_file("input-press-1.txt", "1", false)
  elseif phase == 1 and p.opened_gui_type == defines.gui_type.controller then
    for _, d in ipairs(seen) do
      if not kind and not d.blocked and d.player == 1 and d.type:find("Open") then kind = d.type end
    end
    say((kind and "PASS" or "FAIL") .. " pressing E arrived as an action: " .. tostring(kind))
    local types = {}
    for _, d in ipairs(seen) do types[#types + 1] = d.type end
    say("  actions seen: " .. table.concat(types, ", "))
    p.opened = nil
    native.call("input", "block", helpers.table_to_json({ types = { kind or "none" } }))
    phase = 2
    seen = {}
    helpers.write_file("input-press-2.txt", "1", false)
    phase_tick = e.tick
  elseif phase == 2 and e.tick > phase_tick + 120 then
    local blocked = false
    for _, d in ipairs(seen) do if d.type == kind and d.blocked then blocked = true end end
    say((blocked and "PASS" or "FAIL") .. " the second press, blocked, arrived with blocked = true")
    say((p.opened_gui_type ~= defines.gui_type.controller and "PASS" or "FAIL") .. " a blocked action does nothing")
    native.call("input", "block", '{"types": []}')
    say("  status: " .. tostring(native.call("input", "status")))
    -- B3: the same action sent from Lua through the game's input pipeline
    local ok, err = native.send_action("OpenCharacterGui")
    say((ok and "PASS" or "FAIL") .. " send_action accepted: " .. tostring(err))
    phase = 4
    phase_tick = e.tick
  elseif phase == 4 and e.tick > phase_tick + 30 then
    say((p.opened_gui_type == defines.gui_type.controller and "PASS" or "FAIL") .. " a sent action opens the inventory")
    p.opened = nil
    -- hook presets
    for _, name in ipairs({ "console", "save", "expansion", "app-state" }) do
      local ev, err = native.call("hooks", "add", helpers.table_to_json({ preset = name }))
      say((ev == name and "PASS" or "FAIL") .. " preset " .. name .. ": " .. tostring(ev or err))
    end
    local ms = game.map_settings.enemy_expansion
    ms.enabled, ms.min_expansion_cooldown, ms.max_expansion_cooldown = true, 60, 60
    game.print("fse preset test")
    game.auto_save("fse-preset")
    phase = 5
    phase_tick = e.tick
  elseif phase == 5 and (e.tick > phase_tick + 3600 or (sim_seen.console and sim_seen.expansion and local_seen.save)) then
    say((sim_seen.console and "PASS" or "FAIL") .. " console preset: " .. serpent.line(sim_seen.console))
    say((local_seen.save and "PASS" or "FAIL") .. " save preset (local): " .. serpent.line(local_seen.save))
    say((sim_seen.expansion and "PASS" or "SKIP") .. " expansion preset: " .. serpent.line(sim_seen.expansion))
    -- prototype tooltip rows: Factoriopedia describes iron plate
    native.call("entityinfo", "set_proto", helpers.table_to_json({ type = "item", name = "iron-plate",
      rows = { { "FSE test", "a row of a mod's own" } } }))
    p.open_factoriopedia_gui(prototypes.item["iron-plate"])
    phase = 6
    phase_tick = e.tick
  elseif phase == 6 and e.tick > phase_tick + 90 then
    game.take_screenshot({ player = p, path = "input-tooltip.png", show_gui = true })
    local st = helpers.json_to_table(native.call("entityinfo", "status") or "{}") or {}
    say(((st.proto_shown or 0) > 0 and "PASS" or "FAIL") .. " prototype rows shown: " .. serpent.line(st))
    phase = 7
    finish()
  elseif phase == 1 and e.tick > 60 + 600 then
    say("FAIL the inventory never opened (the key press didn't reach the game)")
    phase = 3
    finish()
  end
end)
