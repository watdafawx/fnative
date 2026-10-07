-- Several handler tables, one registration per event.
--
-- script.on_event replaces whatever handler an event had, so a mod using the library hands it all its handlers
-- together with the library's:
--   local events = require("__fnative-std__/events")
--   events.register({ input.handlers, window.handlers, dnd.handlers, my_handlers })
-- where each table is { [defines.events.on_tick] = function(e) ... end, ... }. Handlers of one event run in the
-- order of the tables.

local safe = require("__fnative-std__/safe")

local M = {}

-- (every handler runs guarded: an error in a mod's GUI code is logged and skipped, it never ends the game)
function M.register(tables, name)
  name = name or script.mod_name
  local merged, order = {}, {}
  for _, t in ipairs(tables) do
    for ev, fn in pairs(t) do
      if not merged[ev] then
        merged[ev] = {}
        order[#order + 1] = ev
      end
      table.insert(merged[ev], fn)
    end
  end
  for _, ev in ipairs(order) do
    local fns = merged[ev]
    if #fns == 1 then
      script.on_event(ev, safe.guard(name, fns[1]))
    else
      local guarded = {}
      for i = 1, #fns do guarded[i] = safe.guard(name, fns[i]) end
      script.on_event(ev, function(e)
        for i = 1, #guarded do guarded[i](e) end
      end)
    end
  end
end

return M
