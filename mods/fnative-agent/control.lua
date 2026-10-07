-- fnative-agent: characters without a player, for an external program (an AI agent) to drive.
--
-- remote.call("agent", ...), or over HTTP with the fnative loader + bridge:
--   POST /api/game {"interface": "agent", "function": "walk_to", "args": ["bob", 10, -5]}
--
--   spawn(name, x?, y?, surface?)   a character near the first player (or at x, y); returns its status
--   remove(name)
--   list()                          names and positions
--   status(name)                    position, health, inventory, what it is doing
--   look(name, radius?)             entities around it (up to 150, nearest first) and resources by amount
--   walk_to(name, x, y)             walks there (straight line, 8 directions); events: arrived / stuck
--   stop(name)
--   mine(name, x, y)                mines what is at x, y (an entity or a resource tile) until gone or 30 s; events
--   craft(name, recipe, count?)     hand-crafts; returns how many crafts started
--   place(name, item, x, y, direction?)   builds from its own inventory, within its build reach
--   give(name, item, count)         cheat: puts items in its inventory (for testing)
--   say(name, text)                 chat + a speech line above it for 4 s
--   events(name)                    what happened since the last call (arrived, stuck, mined, ...), then forgets them
-- Positions are {x, y} tables in answers.

local D = defines.direction
local DIRS = { D.north, D.northeast, D.east, D.southeast, D.south, D.southwest, D.west, D.northwest }

local function agents()
  storage.agents = storage.agents or {}
  return storage.agents
end

local function get(name)
  local a = agents()[name]
  if not a then error("no agent " .. tostring(name)) end
  if not (a.entity and a.entity.valid) then
    agents()[name] = nil
    error("agent " .. name .. " is gone (died or removed)")
  end
  return a
end

local function event(a, kind, data)
  a.events = a.events or {}
  data = data or {}
  data.kind, data.tick = kind, game.tick
  a.events[#a.events + 1] = data
  if #a.events > 100 then table.remove(a.events, 1) end
end

local function pos(p) return { x = math.floor(p.x * 100 + 0.5) / 100, y = math.floor(p.y * 100 + 0.5) / 100 } end

local function inventory(e)
  local inv = e.get_main_inventory()
  local out = {}
  for _, c in pairs(inv and inv.get_contents() or {}) do out[c.name] = (out[c.name] or 0) + c.count end
  return out
end

local function status(name)
  local a = get(name)
  local e = a.entity
  return { name = name, position = pos(e.position), surface = e.surface.name, health = e.health,
    inventory = inventory(e), walking_to = a.target and pos(a.target) or nil,
    mining = a.mine and pos(a.mine.position) or nil, crafting_queue = e.crafting_queue_size }
end

local api = {}

function api.spawn(name, x, y, surface)
  if agents()[name] and agents()[name].entity and agents()[name].entity.valid then error("agent " .. name .. " exists") end
  local player = game.connected_players[1] or game.players[1]
  local s = game.surfaces[surface or (player and player.surface.name) or "nauvis"]
  local at = (x and y) and { x = x, y = y } or (player and { x = player.position.x + 3, y = player.position.y }) or { x = 0, y = 0 }
  at = s.find_non_colliding_position("character", at, 20, 0.5) or at
  local e = s.create_entity({ name = "character", position = at, force = player and player.force or "player" })
  if not e then error("could not place a character at " .. at.x .. "," .. at.y) end
  agents()[name] = { entity = e, events = {} }
  return status(name)
end

function api.remove(name)
  local a = agents()[name]
  if a and a.entity and a.entity.valid then a.entity.destroy() end
  agents()[name] = nil
  return true
end

function api.list()
  local out = {}
  for n, a in pairs(agents()) do
    if a.entity and a.entity.valid then out[n] = pos(a.entity.position) end
  end
  return out
end

api.status = status

function api.look(name, radius)
  local e = get(name).entity
  radius = math.min(radius or 16, 64)
  local found = e.surface.find_entities_filtered({ position = e.position, radius = radius })
  local p = e.position
  table.sort(found, function(u, v)
    return (u.position.x - p.x) ^ 2 + (u.position.y - p.y) ^ 2 < (v.position.x - p.x) ^ 2 + (v.position.y - p.y) ^ 2
  end)
  local entities, resources = {}, {}
  for _, f in ipairs(found) do
    if f.type == "resource" then
      resources[f.name] = (resources[f.name] or 0) + f.amount
    elseif f ~= e and #entities < 150 then
      entities[#entities + 1] = { name = f.name, type = f.type, position = pos(f.position), force = f.force.name }
    end
  end
  return { position = pos(p), radius = radius, entities = entities, resources = resources }
end

function api.walk_to(name, x, y)
  local a = get(name)
  a.target, a.last_pos, a.still = { x = x, y = y }, nil, 0
  return true
end

function api.stop(name)
  local a = get(name)
  a.target, a.mine = nil, nil
  a.entity.walking_state = { walking = false, direction = D.north }
  a.entity.mining_state = { mining = false }
  return true
end

function api.mine(name, x, y)
  local a = get(name)
  a.mine = { position = { x = x, y = y }, until_tick = game.tick + 1800 }
  return true
end

function api.craft(name, recipe, count)
  return get(name).entity.begin_crafting({ recipe = recipe, count = count or 1 })
end

function api.place(name, item, x, y, direction)
  local e = get(name).entity
  local proto = prototypes.item[item]
  if not (proto and proto.place_result) then error(tostring(item) .. " can't be placed") end
  local dx, dy = x - e.position.x, y - e.position.y
  if dx * dx + dy * dy > e.build_distance ^ 2 then error("out of reach (" .. e.build_distance .. " tiles): walk closer") end
  local inv = e.get_main_inventory()
  if inv.get_item_count(item) < 1 then error("no " .. item .. " in its inventory") end
  local spec = { name = proto.place_result.name, position = { x = x, y = y }, direction = direction or D.north,
    force = e.force }
  if not e.surface.can_place_entity(spec) then error("something is in the way at " .. x .. "," .. y) end
  spec.raise_built = true
  local built = e.surface.create_entity(spec)
  if not built then error("could not build") end
  inv.remove({ name = item, count = 1 })
  return { name = built.name, position = pos(built.position) }
end

function api.give(name, item, count)
  return get(name).entity.insert({ name = item, count = count or 1 })
end

function api.say(name, text)
  local e = get(name).entity
  game.print("[" .. name .. "] " .. tostring(text))
  rendering.draw_text({ text = tostring(text), surface = e.surface, target = { entity = e, offset = { 0, -2.5 } },
    color = { 1, 1, 0.6 }, scale = 1.2, alignment = "center", time_to_live = 240 })
  return true
end

function api.events(name)
  local a = get(name)
  local out = a.events or {}
  a.events = {}
  return out
end

remote.add_interface("agent", api)

-- steering and mining, every tick, for the agents that have something to do
-- (guarded: a bad command or answer is logged and skipped, never the end of the game)
local function guarded(fn)
  return function(e)
    local ok, err = pcall(fn, e)
    if not ok then log("[" .. script.mod_name .. "] error (handled): " .. tostring(err)) end
  end
end
script.on_event(defines.events.on_tick, guarded(function()
  for name, a in pairs(storage.agents or {}) do
    local e = a.entity
    if not (e and e.valid) then
      storage.agents[name] = nil
    else
      if a.target then
        local dx, dy = a.target.x - e.position.x, a.target.y - e.position.y
        if dx * dx + dy * dy < 0.36 then
          a.target = nil
          e.walking_state = { walking = false, direction = D.north }
          event(a, "arrived", { position = pos(e.position) })
        else
          local i = math.floor(math.atan2(dx, -dy) / (math.pi / 4) + 0.5) % 8
          e.walking_state = { walking = true, direction = DIRS[i + 1] }
          -- (stuck: hardly moved for a second)
          if a.last_pos and (e.position.x - a.last_pos.x) ^ 2 + (e.position.y - a.last_pos.y) ^ 2 < 0.0001 then
            a.still = (a.still or 0) + 1
            if a.still > 60 then
              a.target = nil
              e.walking_state = { walking = false, direction = D.north }
              event(a, "stuck", { position = pos(e.position) })
            end
          else
            a.still = 0
          end
          a.last_pos = { x = e.position.x, y = e.position.y }
        end
      end
      if a.mine then
        local m = a.mine
        local target = e.surface.find_entities_filtered({ position = m.position, radius = 0.5, limit = 1 })[1]
        if not target or game.tick > m.until_tick then
          e.mining_state = { mining = false }
          event(a, target and "mining-timeout" or "mined", { position = pos(m.position) })
          a.mine = nil
        else
          e.update_selected_entity(m.position)
          e.mining_state = { mining = true, position = m.position }
        end
      end
    end
  end
end))
