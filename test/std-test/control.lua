-- drives fnative-std-demo in a real client with a mocked mouse (std.mock) and screenshots the steps
-- (no crash-site intro: it pauses the game and waits for the player to press Tab)
script.on_init(function()
  local fp = remote.interfaces["freeplay"]
  if fp then
    if fp.set_skip_intro then remote.call("freeplay", "set_skip_intro", true) end
    if fp.set_disable_crashsite then remote.call("freeplay", "set_disable_crashsite", true) end
  end
end)

local out = {}
local function say(s) out[#out + 1] = s end
local function shot(name)
  game.take_screenshot({ player = 1, show_gui = true, path = "fstd-" .. name .. ".png", resolution = { game.get_player(1).display_resolution.width, game.get_player(1).display_resolution.height }, zoom = 1 })
end
local function mock(t) native.call("std", "mock", helpers.table_to_json(t)) end
local D = function(fn, ...) return remote.call("fnative-std-demo", fn, ...) end
local steps = {
  [60] = function() D("open", 1); mock({ left = false, x = 200, y = 200, focused = true }); say("order before: " .. D("order", 1)) end,
  [70] = function() D("hover_slot", 1, 1) end,
  [72] = function() mock({ left = true }) end,
  [75] = function() D("hover_slot", 1, 5); mock({ x = 420, y = 330 }) end,
  [80] = function() local g = D("ghost", 1); say("ghost while dragging at " .. (g and (g.x .. "," .. g.y) or "none")); shot("1-drag") end,
  [85] = function() mock({ left = false }) end,
  [90] = function() say("order after drop on slot 5: " .. D("order", 1)); say("ghost after drop: " .. tostring(D("ghost", 1))); shot("2-dropped") end,
  [95] = function() D("hover_slot", 1, 2) end,
  [97] = function() mock({ left = true }) end,
  [99] = function() mock({ left = false }) end,
  [102] = function() say("press+release on one slot (a click): order " .. D("order", 1)) end,
  [110] = function() D("hover_grip", 1); mock({ left = false, x = 500, y = 500 }) end,
  [112] = function() mock({ left = true }) end,
  [116] = function() mock({ x = 700, y = 600 }) end,
  [120] = function() mock({ left = false }); local s = D("size", 1); say("size after dragging the grip +200,+100 px: " .. s.w .. " x " .. s.h .. " (scale " .. game.get_player(1).display_scale .. ")") end,
  [125] = function() shot("3-resized") end,
  [140] = function() native.call("std", "mock", ""); helpers.write_file("fstd-result.txt", table.concat(out, "\n") .. "\n", false); helpers.write_file("fstd-done.txt", "1", false) end,
}
script.on_event(defines.events.on_tick, function(e)
  if not native then if e.tick == 60 then helpers.write_file("fstd-result.txt", "no native\n", false); helpers.write_file("fstd-done.txt", "1", false) end return end
  local f = steps[e.tick]
  if f then f() end
end)
