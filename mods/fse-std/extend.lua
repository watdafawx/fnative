-- New properties and methods on the game's own Lua classes (LuaEntity, LuaPlayer, LuaSurface, ...), in this mod's
-- Lua state (other mods don't see them):
--
--   local extend = require("__fse-std__/extend")
--   extend.class(any_entity, {
--     fse_held_count = function(self) return native.read(self, "entityTarget.target.heldStack.count") end,
--     fse_say = extend.method(function(self, text) game.print(self.name .. ": " .. text) end),
--   })
--   entity.fse_held_count; entity:fse_say("hi")
--
-- A function value is a property: called with the object on every read. extend.method makes it a method. `sample` is
-- any object of the class (all objects of a class share its metatable). Names should start with your own prefix.
-- Once a class is extended, every read of its fields goes through one more Lua function in this mod.
-- After loading a save the game's metatables are new: extend again (on_load, or lazily). Without fse: false.

local M = {}

function M.class(sample, props)
  if not native or not native.metatable then return false end
  local mt = native.metatable(sample)
  if type(mt) ~= "table" then return false end
  local ext = rawget(mt, "__fse_ext")
  if not ext then
    ext = {}
    local orig = mt.__index
    rawset(mt, "__fse_ext", ext)
    rawset(mt, "__index", function(self, k)
      local f = ext[k]
      if f ~= nil then return f(self) end
      if type(orig) == "function" then return orig(self, k) end
      return orig[k]
    end)
  end
  for k, v in pairs(props) do ext[k] = v end
  return true
end

function M.method(fn)
  return function(self)
    return function(_, ...) return fn(self, ...) end
  end
end

return M
