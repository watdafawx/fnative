-- writes script-output/engine-api.txt: PASS/FAIL lines for native.read, layout, metatable, events, hooks, extend
local nev = require("__fse-std__/native_events")
local extend = require("__fse-std__/extend")
local lines, failed = {}, 0
local function check(name, ok, detail)
  if not ok then failed = failed + 1 end
  lines[#lines + 1] = (ok and "PASS " or "FAIL ") .. name .. (detail and (" :: " .. serpent.line(detail)) or "")
end

local function run()
  if not native then
    lines[#lines + 1] = "FAIL no native table"
    return
  end
  local s = game.surfaces[1]
  local ins = s.create_entity({ name = "inserter", position = { 10.5, 7.5 }, force = "player" })
  storage.ins = ins
  local chest = s.create_entity({ name = "iron-chest", position = { -3.5, 4.5 }, force = "player" })

  local t, err = native.read(ins, "entityTarget.target")
  check("entity target is the real class", t and t.type == "Inserter", t or err)
  local pos = t and native.read(t, "position")
  check("position", pos and pos.x == 10.5 and pos.y == 7.5, pos)
  local name = t and native.read(t, "prototype.name")
  check("prototype name through a pointer and bases", name == "inserter", name)
  local held = t and native.read(t, "heldStack.count")
  check("a field of a nested object", held == 0, held)
  local whole = t and native.read(t, "heldStack", 1)
  check("a nested object as a table", type(whole) == "table" and whole.count == 0, whole)
  local cpos = native.read(chest, "entityTarget.target.position")
  check("one path through the Lua object", cpos and cpos.x == -3.5 and cpos.y == 4.5, cpos)
  local up = t and native.read(t, "^Entity.position")
  check("viewed as a base class", up and up.x == 10.5, up)
  local surf = native.read(s, "surfaceTarget.target")
  check("a surface", surf and surf.type == "Surface", surf)

  local layout = native.layout("Inserter")
  check("layout", layout and layout.size > 0 and #layout.fields > 3, layout and layout.size)
  local bad, berr = native.read({ ptr = 16, type = "Entity" }, "position")
  check("a bad address is an error, not a crash", bad == nil and type(berr) == "string", berr)
  local nofield, nerr = native.read(t or {}, "noSuchField")
  check("an unknown field is an error", nofield == nil and nerr ~= nil, nerr)

  local mt = native.metatable(ins)
  check("metatable", type(mt) == "table", type(mt))
  if type(mt) == "table" then
    local orig = mt.__index
    mt.__index = function(self, k)
      if k == "fse_raw_position" then return native.read(self, "entityTarget.target.position") end
      if type(orig) == "function" then return orig(self, k) end
      return orig[k]
    end
    local ok, v = pcall(function() return chest.fse_raw_position end)
    check("a new property on LuaEntity", ok and v and v.x == -3.5, ok and v or tostring(v))
    check("the old properties still work", ins.name == "inserter" and chest.position.x == -3.5)
  end

  local list, newest = native.events()
  check("events: subscribe", type(list) == "table" and #list == 0 and type(newest) == "number", newest)

  -- hooks: Entity::rotate (this, the hidden ActionResult, the direction) as an event, through fse-std's poller
  local got
  nev.on("hooks", "test-rotate", function(data) got = data end)
  nev.poll()
  local spec = { fn = "?rotate@Entity@@UEAA?AVActionResult@@W4RotateDirection@@@Z", event = "test-rotate",
                 args = { { arg = 0, path = "prototype.name", as = "name" }, { arg = 0, path = "position", as = "pos" },
                          { arg = 2, raw = true, as = "dir" } } }
  local ev, herr = native.call("hooks", "add", helpers.table_to_json(spec))
  check("hooks.add", ev == "test-rotate", herr)
  ins.rotate()
  nev.poll()
  check("a hooked call arrives as an event", got and got.args.name == "inserter" and got.args.pos.x == 10.5
        and got.calls == 1, got)
  native.call("hooks", "disable", helpers.table_to_json({ event = "test-rotate" }))
  got = nil
  ins.rotate()
  nev.poll()
  check("a disabled hook sends nothing", got == nil, got)
  local st = helpers.json_to_table(native.call("hooks", "status") or "{}")
  check("hooks.status counts calls", st and st.hooks and st.hooks[1] and st.hooks[1].calls == 2, st)
  local _, dup = native.call("hooks", "add", helpers.table_to_json(spec))
  check("the same function twice is refused", dup ~= nil, dup)
  local _, flt = native.call("hooks", "add", helpers.table_to_json({ fn = "?noSuchFunction@Nothing@@QEAAXXZ", event = "f" }))
  check("a function this build lacks is refused", flt ~= nil, flt)

  -- extend: a property and a method on LuaEntity
  check("extend.class", extend.class(chest, {
    fse_x = function(self) return native.read(self, "entityTarget.target.position").x end,
    fse_twice = extend.method(function(self, n) return n * 2 end),
  }))
  check("an extended property", ins.fse_x == 10.5, ins.fse_x)
  check("an extended method", chest:fse_twice(21) == 42)
  check("game fields after extending", ins.name == "inserter" and ins.valid)
end

-- simulation events: the rotate hook fired inside the tick's update, so "fse-event" brings it at that tick's end
local sim, sim_tick = {}, nil
script.on_event("fse-event", function(e)
  for _, ev in ipairs(e.events) do sim[#sim + 1] = ev end
  sim_tick = sim_tick or game.tick
end)

script.on_event(defines.events.on_tick, function(e)
  if e.tick == 2 then
    local ok, err = pcall(run)
    if not ok then check("no Lua error", false, err) end
  elseif e.tick == 3 then
    local first = sim[1]
    check("a hooked call in the update arrives as fse-event before the next tick (game.tick has moved on)", sim_tick == 3 and first
          and first.plugin == "hooks" and first.name == "test-rotate" and first.data.args.name == "inserter",
          { tick = sim_tick, first = first })
    local _, local_err = native.sync("x", "y")
    check("native.sync needs a player (headless has none)", local_err ~= nil, local_err)
    check("no local player headless", native.local_player() == nil)
    local root = native.root()
    check("root objects", root and root.game and root.game.type == "Game" and root.map and root.map.type == "Map"
          and root.local_player == nil, root)
    local ts = native.tick_stats()
    check("tick stats", ts and ts.last_ms >= 0 and ts.avg_ms >= 0, ts)
    local ins = storage.ins
    local w, werr = native.write(ins, "entityTarget.target.heldStack.count", 0)
    check("write a number (and read it back)", w and native.read(ins, "entityTarget.target.heldStack.count") == 0, werr)
    local _, nerr = native.write(ins, "entityTarget.target.position", 1)
    check("writing a non-number is refused", nerr ~= nil, nerr)
    local _, serr = native.send_action("OpenCharacterGui")
    check("send_action needs a player", serr ~= nil, serr)
    lines[#lines + 1] = failed == 0 and "ALL PASS" or (failed .. " FAILED")
    helpers.write_file("engine-api.txt", table.concat(lines, "\n") .. "\n", false)
  end
end)
