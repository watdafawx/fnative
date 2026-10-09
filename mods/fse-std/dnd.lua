-- Drag & drop between GUI elements.
--
--   dnd.draggable(element, payload)      payload: anything JSON-like, handed to the drop handlers;
--                                        a table's .sprite / .caption is what the ghost shows; with .card_size (px)
--                                        the ghost is a copy of the element, centred on the cursor
--   dnd.droppable(element, accept?)      accept: true (anything), or a kind: only payloads with .kind == accept
--   dnd.on_drop(function(player, payload, target, source) ... end)
--   events.register({ input.handlers, dnd.handlers, ... })
--
-- With the fse std plugin: press on a draggable, move off it with the button held (a ghost follows the cursor,
-- the droppable under it lights up), release over a droppable. A press and release on the same element stays a
-- plain click (on_gui_click as usual).
-- Without it: click a draggable to pick it up (a "carrying" note shows), click a droppable to drop, right-click to
-- cancel. Mods using clicks on the same elements should skip those dnd used:  if dnd.handled(e) then return end
-- Drags are momentary (not saved); a ghost left in a save is cleaned up.

local input = require("__fse-std__/input")

local M = {}
local DRAG, DROP = "fstd_drag", "fstd_drop"
local drop_handlers, phase_handlers = {}, {}
local state = {}       -- player_index -> {phase = "armed" | "dragging" | "picked", source, payload, ghost, target}
local last_left = {}
local handled_tick = {}  -- player_index -> tick of a click dnd used

function M.draggable(el, payload)
  local t = el.tags
  t[DRAG] = payload == nil and true or payload
  el.tags = t
  el.raise_hover_events = true
end

function M.droppable(el, accept)
  local t = el.tags
  t[DROP] = accept == nil and true or accept
  el.tags = t
  el.raise_hover_events = true
end

function M.on_drop(fn) drop_handlers[#drop_handlers + 1] = fn end

--- on_phase(function(player, phase, payload, source, target, dropped) ... end), for a payload with .preview = true:
--- "start" (the ghost appears), "over" (target changed; nil = off the droppables), "end" (after the drop handlers).
--- The target is the one a release would drop on, so a mod can show the result already (a live reorder).
function M.on_phase(fn) phase_handlers[#phase_handlers + 1] = fn end

function M.dragging(player_index)
  local s = state[player_index]
  return s ~= nil and s.phase ~= "armed"
end

function M.handled(e)
  return handled_tick[e.player_index] == game.tick
end

local function accepts(el, payload)
  if not (el and el.valid) then return false end
  local a = el.tags[DROP]
  if a == nil or a == false then return false end
  if a == true then return true end
  return type(payload) == "table" and payload.kind == a
end

local function highlight(s, el)
  if s.target and s.target.valid and s.target ~= el then pcall(function() s.target.toggled = false end) end
  s.target = el
  if el and el.valid then pcall(function() el.toggled = true end) end
end

-- a copy of an element and everything in it, for a ghost (styles are copied by name, so it looks the same)
local function clone(parent, el)
  local def = { type = el.type, style = el.style.name, ignored_by_interaction = true }
  for _, k in ipairs({ "caption", "sprite", "value", "visible", "toggled", "direction" }) do
    local ok, v = pcall(function() return el[k] end)
    if ok and v ~= nil and v ~= "" then def[k] = v end
  end
  if el.type == "sprite-button" or el.type == "button" then def.toggled = nil end
  local c = parent.add(def)
  if el.type == "flow" then
    local ok, v = pcall(function() return el.style.horizontal_spacing end)
    if ok and v then c.style.horizontal_spacing = v end
  end
  for _, child in ipairs(el.children) do clone(c, child) end
  return c
end

local function make_ghost(player, s, note)
  local size = type(s.payload) == "table" and s.payload.card_size
  if size and not note then  -- a card: a copy of the source, centred on the cursor
    local g = player.gui.screen.add({ type = "flow", name = "fstd_dnd_ghost", ignored_by_interaction = true })
    clone(g, s.source)
    s.ghost, s.ghost_offset = g, -size / 2
    return g
  end
  local g = player.gui.screen.add({ type = "frame", name = "fstd_dnd_ghost", style = "fstd_ghost", direction = "horizontal",
    ignored_by_interaction = true })
  local src = s.source
  local sprite = type(s.payload) == "table" and s.payload.sprite  -- (for sources that draw their icon in child elements)
  if not sprite and src.valid and src.type == "sprite-button" and src.sprite and src.sprite ~= "" then sprite = src.sprite end
  if sprite then g.add({ type = "sprite", sprite = sprite, ignored_by_interaction = true }) end
  local cap = src.valid and src.caption
  if type(s.payload) == "table" and s.payload.caption then cap = s.payload.caption end
  if cap and cap ~= "" then g.add({ type = "label", caption = cap, ignored_by_interaction = true }) end
  if note then g.add({ type = "label", caption = note, style = "info_label", ignored_by_interaction = true }) end
  s.ghost = g
  return g
end

local function previewing(s) return type(s.payload) == "table" and s.payload.preview end

local function emit(player, s, phase, target, dropped)
  if not previewing(s) then return end
  for _, fn in ipairs(phase_handlers) do fn(player, phase, s.payload, s.source, target, dropped) end
end

local function finish(player, s, drop)
  local target, source, payload = s.target, s.source, s.payload
  highlight(s, nil)
  if s.ghost and s.ghost.valid then s.ghost.destroy() end
  state[player.index] = nil
  if drop and target and target.valid and target ~= source and accepts(target, payload) then
    for _, fn in ipairs(drop_handlers) do fn(player, payload, target, source) end
  else
    drop = false
  end
  s.target = target
  emit(player, s, "end", target, drop)
end

-- (a ghost left in a save by a drag that never finished)
local function sweep(player)
  local g = player.gui.screen.fstd_dnd_ghost
  if g and not state[player.index] then g.destroy() end
end

local function tick()
  for _, player in pairs(game.connected_players) do
    local i = player.index
    local s = state[i]
    local h = input.hovered[i]
    if h and not h.valid then
      input.hovered[i] = nil
      h = nil
    end
    if s and s.phase == "picked" then
      -- (click mode: nothing to poll)
    elseif s or (h and h.tags[DRAG] ~= nil) then
      local m = input.this_tick()
      if m then
        if not s then
          if m.left and last_left[i] == false then  -- a press that began over the draggable
            state[i] = { phase = "armed", source = h, payload = h.tags[DRAG] }
          end
        elseif s.phase == "armed" then
          if not m.left then
            state[i] = nil  -- released over it: a plain click
          elseif h ~= s.source then
            s.phase = "dragging"
            make_ghost(player, s)
            emit(player, s, "start")
          end
        end
        s = state[i]
        if s and s.phase == "dragging" then
          if not s.source.valid then
            finish(player, s, false)
          elseif not m.left then
            -- (the hover event for what's under the cursor can arrive on the release tick itself: use it)
            if accepts(h, s.payload) and h ~= s.source then highlight(s, h) end
            finish(player, s, true)
          else
            if s.ghost and s.ghost.valid then local o = s.ghost_offset or 16; s.ghost.location = { x = m.x + o, y = m.y + o } end
            if previewing(s) then
              -- (the card slides under the cursor, so the cursor is often over the source itself, or between two
              -- cards: keep the target until the cursor reaches another droppable or leaves them all)
              local t = s.target
              if h and h ~= s.source then
                t = accepts(h, s.payload) and h or nil
              end
              if t ~= s.target then
                s.target = t
                emit(player, s, "over", t)
              end
            else
              highlight(s, accepts(h, s.payload) and h or nil)
            end
          end
        end
        last_left[i] = m.left
      end
    else
      last_left[i] = false
      if game.tick % 600 == 0 then sweep(player) end
    end
  end
end

-- without the plugin: click to pick, click to drop, right-click to cancel
local function click(e)
  if input.native() then return end
  local el = e.element
  if not (el and el.valid) then return end
  local player = game.get_player(e.player_index)
  local s = state[e.player_index]
  if s and s.phase == "picked" then
    handled_tick[e.player_index] = game.tick
    if e.button == defines.mouse_button_type.right or el == s.source then
      return finish(player, s, false)
    end
    if accepts(el, s.payload) then
      s.target = el
      return finish(player, s, true)
    end
    return
  end
  if el.tags[DRAG] ~= nil and e.button == defines.mouse_button_type.left and (e.shift or e.control or e.alt) == false then
    handled_tick[e.player_index] = game.tick
    s = { phase = "picked", source = el, payload = el.tags[DRAG] }
    state[e.player_index] = s
    local g = make_ghost(player, s, "click where to drop it, right-click to cancel")
    g.location = { x = 40, y = 40 }
  end
end

M.handlers = {
  [defines.events.on_tick] = function()
    if input.native() then tick() end
  end,
  [defines.events.on_gui_click] = click,
}

return M
