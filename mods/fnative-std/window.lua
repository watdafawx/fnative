-- Windows for mod GUIs: a title bar to move them by, a close button, a corner to resize them by (with the fnative
-- std plugin), and the position and size each player left them at.
--
--   local frame, content = window.create(player, { name = "my_window", title = "My window",
--     width = 400, height = 300, resizable = true, close = true, min_width = 200, min_height = 120 })
--   content.add{...}                       -- your elements
--   window.on_close(function(player, name) ... end)
--   window.on_resize(function(player, name, width, height) ... end)   -- while the grip is dragged (GUI units)
--   events.register({ input.handlers, window.handlers, ... })
-- Sizes are in GUI units (as style.width): the window is display_scale times that in pixels. Without a size the
-- window is 400 x 300.
--   window.size(player, name) -> width, height   (what the library set: Lua can't read a style's size back)

local input = require("__fnative-std__/input")

local M = {}
local closers = {}
local resizers = {}
local resizing = {}  -- player_index -> {name, x, y, w, h}  (momentary: not saved)
local last_left = {}

local function saved(player_index, name)
  storage.fstd_windows = storage.fstd_windows or {}
  local p = storage.fstd_windows[player_index] or {}
  storage.fstd_windows[player_index] = p
  p[name] = p[name] or {}
  return p[name]
end

function M.on_close(fn) closers[#closers + 1] = fn end
function M.on_resize(fn) resizers[#resizers + 1] = fn end

function M.size(player, name)
  local s = saved(player.index, name)
  return s.width, s.height
end

function M.create(player, opts)
  local name = assert(opts.name, "window.create: name is required")
  local screen = player.gui.screen
  if screen[name] then screen[name].destroy() end
  local frame = screen.add({ type = "frame", name = name, direction = "vertical", tags = { fstd_window = name } })
  local bar = frame.add({ type = "flow", direction = "horizontal" })
  bar.drag_target = frame
  bar.style.horizontal_spacing = 8
  bar.add({ type = "label", caption = opts.title or name, style = "frame_title", ignored_by_interaction = true })
  local filler = bar.add({ type = "empty-widget", style = "draggable_space_header", ignored_by_interaction = true })
  filler.style.height = 24
  filler.style.horizontally_stretchable = true
  if opts.close ~= false then
    bar.add({ type = "sprite-button", style = "frame_action_button", sprite = "utility/close",
      tags = { fstd_close = name }, tooltip = { "gui.close" } })
  end
  local content = frame.add({ type = "frame", style = "inside_shallow_frame_with_padding", direction = "vertical" })
  content.style.horizontally_stretchable = true
  content.style.vertically_stretchable = true
  local s = saved(player.index, name)
  -- (a style's width can be set, not read: the window always gets a size, and the library keeps it)
  s.width = s.width or opts.width or 400
  s.height = s.height or opts.height or 300
  frame.style.width = s.width
  frame.style.height = s.height
  if opts.resizable ~= false and input.native() then
    local foot = frame.add({ type = "flow", direction = "horizontal" })
    local push = foot.add({ type = "empty-widget", ignored_by_interaction = true })
    push.style.horizontally_stretchable = true
    local grip = foot.add({ type = "empty-widget", style = "fstd_resize_grip", raise_hover_events = true,
      tags = { fstd_resize = name, min_w = opts.min_width or 160, min_h = opts.min_height or 100 },
      tooltip = "Drag to resize" })
    grip.style.top_margin = -4
  end
  if opts.remember ~= false and s.location then
    frame.location = s.location
  else
    frame.force_auto_center()
  end
  return frame, content
end

M.handlers = {
  [defines.events.on_gui_click] = function(e)
    local el = e.element
    if not (el and el.valid) then return end
    local name = el.tags.fstd_close
    if not name then return end
    local player = game.get_player(e.player_index)
    local frame = player.gui.screen[name]
    if frame then frame.destroy() end
    for _, fn in ipairs(closers) do fn(player, name) end
  end,
  [defines.events.on_gui_location_changed] = function(e)
    local el = e.element
    if el and el.valid and el.tags.fstd_window then
      saved(e.player_index, el.tags.fstd_window).location = el.location
    end
  end,
  -- resizing: press on the grip, the window follows the cursor until the button goes up
  [defines.events.on_tick] = function()
    if not input.native() then return end
    for _, player in pairs(game.connected_players) do
      local i = player.index
      local r = resizing[i]
      local h = input.hovered[i]
      local grip = h and h.valid and h.tags.fstd_resize
      if r or grip then
        local m = input.this_tick()
        if m then
          if not r then
            if m.left and not last_left[i] then
              if player.gui.screen[grip] then
                local s = saved(i, grip)
                resizing[i] = { name = grip, x = m.x, y = m.y, w = s.width, h = s.height,
                  min_w = h.tags.min_w, min_h = h.tags.min_h }
              end
            end
          else
            local frame = player.gui.screen[r.name]
            if not (frame and frame.valid) or not m.left then
              resizing[i] = nil
            else
              local scale = player.display_scale
              local s = saved(i, r.name)
              s.width = math.max(r.min_w, math.floor(r.w + (m.x - r.x) / scale))
              s.height = math.max(r.min_h, math.floor(r.h + (m.y - r.y) / scale))
              frame.style.width = s.width
              frame.style.height = s.height
              for _, fn in ipairs(resizers) do fn(player, r.name, s.width, s.height) end
            end
          end
          last_left[i] = m.left
        end
      else
        last_left[i] = nil
      end
    end
  end,
}

return M
