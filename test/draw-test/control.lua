-- the draw plugin in a real client: shapes around a chest and the player; the test script grabs the screen (the
-- drawing is a window of its own over the game) and this writes draw.status
script.on_init(function()
  local fp = remote.interfaces["freeplay"]
  if fp then
    if fp.set_skip_intro then remote.call("freeplay", "set_skip_intro", true) end
    if fp.set_disable_crashsite then remote.call("freeplay", "set_disable_crashsite", true) end
  end
end)

script.on_event(defines.events.on_tick, function(e)
  local p = game.get_player(1)
  if not p then return end
  if e.tick == 60 then
    local c = p.position
    local x, y = math.floor(c.x) + 4.5, math.floor(c.y) + 0.5
    p.surface.create_entity({ name = "iron-chest", position = { x, y }, force = "player" })
    native.call("draw", "set", helpers.table_to_json({ id = "test", surface = p.surface.index, shapes = {
      { rect = { { x - 0.5, y - 0.5 }, { x + 0.5, y + 0.5 } }, color = { 1, 0.2, 0.2 }, width = 3 },
      { circle = { c.x, c.y }, radius = 2, color = { 0.2, 1, 0.2, 0.8 }, width = 2 },
      { line = { { c.x, c.y }, { x, y } }, color = { 0.3, 0.6, 1 }, width = 2 },
      { text = "FSE draw: the chest", at = { x - 0.5, y - 0.8 }, size = 18, color = { 1, 1, 1 } },
      { rect = { { c.x - 6, c.y + 3 }, { c.x - 4, c.y + 4 } }, color = { 1, 0.8, 0, 0.4 }, width = 0 },
    } }))
  elseif e.tick == 180 then
    helpers.write_file("draw-status.txt", native.call("draw", "status") or "no status", false)
  end
end)
