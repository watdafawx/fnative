-- Events native plugins send (native.events): engine hooks, worker threads. Each tick the new ones go to handlers.
--
--   local nev = require("__fse-std__/native_events")
--   nev.on("hooks", "my-event", function(data, ev) ... end)    -- data: the event's data (decoded JSON)
--   events.register({ nev.handlers, my_handlers })              -- (nev.handlers polls on every tick)
--
-- Each mod keeps its own place in the stream, so mods never take events from each other. Events come from outside
-- the simulation and are this peer's own: to change the game with engine-hook events use the "fse-event" event
-- (fse-std's control.lua), the same on every multiplayer peer.
-- Without fse nothing ever arrives.

local M = { handlers = {} }
local subs = {}
local since -- (nil: the first poll only finds where the stream is now)

function M.on(plugin, name, fn)
  subs[plugin .. "\0" .. name] = fn
end

function M.poll()
  if not native or not native.events then return end
  local list, newest = native.events(since)
  since = newest
  for _, ev in ipairs(list) do
    local fn = subs[ev.plugin .. "\0" .. ev.name]
    if fn then fn(ev.data, ev) end
  end
end

M.handlers[defines.events.on_tick] = M.poll

return M
