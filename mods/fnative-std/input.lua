-- What the Lua API doesn't tell a mod, through the fnative loader's std plugin, with nil when it isn't there:
--   input.native()          is the std plugin there?
--   input.read()            {x, y, inside, focused, left, right, middle, shift, ctrl, alt, w, h}: the cursor in the
--                           game window (pixels, the same units as a screen GUI's location) and what is held
--   input.clipboard_get() / input.clipboard_set(text)
--   input.now()             real milliseconds (Lua in the game has no clock)
--   input.read_file(path)   a file under script-output
-- and which element each player's cursor is over (elements need raise_hover_events = true):
--   input.hovered[player_index]   (with input.handlers registered; not saved: hovering is momentary)

local M = {}

local has_std
function M.native()
  if has_std == nil then
    local ok, list = pcall(function() return type(native) == "table" and native.plugins() end)
    has_std = ok and type(list) == "table" and list.std ~= nil
  end
  return has_std
end

local function call(fn, arg)
  if not M.native() then return nil end
  local out = native.call("std", fn, arg or "")
  return out
end

function M.read()
  local s = call("input")
  return s and helpers.json_to_table(s) or nil
end

-- (the library's modules share one read per tick)
local cache, cache_tick
function M.this_tick()
  if cache_tick ~= game.tick then
    cache, cache_tick = M.read(), game.tick
  end
  return cache
end

function M.clipboard_get() return call("clipboard_get") end
function M.clipboard_set(text) return call("clipboard_set", tostring(text)) ~= nil end
function M.now() local s = call("now") return s and tonumber(s) or nil end
function M.read_file(path) return call("read", path) end

M.hovered = {}

M.handlers = {
  [defines.events.on_gui_hover] = function(e) M.hovered[e.player_index] = e.element end,
  [defines.events.on_gui_leave] = function(e)
    if M.hovered[e.player_index] == e.element then M.hovered[e.player_index] = nil end
  end,
}

return M
