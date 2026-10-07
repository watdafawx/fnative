-- /stddemo: a resizable window with items to reorder by drag & drop (the fnative-std library)

local events = require("__fnative-std__/events")
local input = require("__fnative-std__/input")
local window = require("__fnative-std__/window")
local dnd = require("__fnative-std__/dnd")

local START = { "iron-plate", "copper-plate", "steel-plate", "iron-gear-wheel", "electronic-circuit",
  "advanced-circuit", "plastic-bar", "sulfur", "battery", "engine-unit", "processing-unit", "low-density-structure" }

local function order(player)
  storage.order = storage.order or {}
  storage.order[player.index] = storage.order[player.index] or { table.unpack(START) }
  return storage.order[player.index]
end

local function fill(player)
  local frame = player.gui.screen.fstd_demo
  if not frame then return end
  local list = frame.children[2].fstd_demo_scroll.fstd_demo_grid
  list.clear()
  for i, item in ipairs(order(player)) do
    local b = list.add({ type = "sprite-button", sprite = "item/" .. item, style = "slot_button",
      tooltip = { "", "[item=" .. item .. "] ", prototypes.item[item].localised_name }, tags = { slot = i } })
    dnd.draggable(b, { kind = "demo-item", index = i, caption = prototypes.item[item].localised_name })
    dnd.droppable(b, "demo-item")
  end
end

local function open(player)
  local _, content = window.create(player, { name = "fstd_demo", title = "fnative std: drag to reorder",
    width = 360, height = 260, min_width = 220, min_height = 160 })
  content.add({ type = "label", caption = input.native()
    and "Drag an item onto another to move it there. Drag the bottom-right corner to resize."
    or "No fnative loader: click an item, then click where it goes (right-click cancels).", style = "info_label" }).style.single_line = false
  local scroll = content.add({ type = "scroll-pane", name = "fstd_demo_scroll" })
  scroll.style.vertically_stretchable = true
  scroll.add({ type = "table", name = "fstd_demo_grid", column_count = 6 })
  fill(player)
end

dnd.on_drop(function(player, payload, target)
  local list = order(player)
  local from, to = payload.index, target.tags.slot
  local item = table.remove(list, from)
  table.insert(list, to, item)
  fill(player)
end)

commands.add_command("stddemo", "Opens the fnative-std demo window", function(c)
  open(game.get_player(c.player_index))
end)

-- for automated tests: open the window, point "the cursor" at a slot or the resize grip, read the order
local function find(root, pred)
  if pred(root) then return root end
  for _, c in pairs(root.children) do
    local f = find(c, pred)
    if f then return f end
  end
end
remote.add_interface("fnative-std-demo", {
  open = function(pi) open(game.get_player(pi)) end,
  hover_slot = function(pi, slot)
    local frame = game.get_player(pi).gui.screen.fstd_demo
    input.hovered[pi] = slot and find(frame, function(e) return e.tags.slot == slot end) or nil
  end,
  hover_grip = function(pi)
    local frame = game.get_player(pi).gui.screen.fstd_demo
    input.hovered[pi] = find(frame, function(e) return e.tags.fstd_resize ~= nil end)
  end,
  order = function(pi) return table.concat(order(game.get_player(pi)), ",") end,
  size = function(pi) local w, h = window.size(game.get_player(pi), "fstd_demo") return { w = w, h = h } end,
  ghost = function(pi) local g = game.get_player(pi).gui.screen.fstd_dnd_ghost return g and g.location or false end,
})

events.register({ input.handlers, window.handlers, dnd.handlers })
