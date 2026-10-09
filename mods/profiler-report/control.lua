-- test: profiles ticks 60..660 with the fse profiler and writes script-output/profile.json
local started
script.on_event(defines.events.on_tick, function(e)
  if not native then return end
  if not started and e.tick % 60 == 0 then
    native.call("profiler", "profile", "test")  -- (a baseline)
    native.call("profiler", "start")
    started = e.tick
  elseif started and e.tick == started + 600 then
    local out = native.call("profiler", "profile", "test")
    native.call("profiler", "stop")
    helpers.write_file("profile.json", out or "null", false)
    helpers.write_file("profile-status.json", native.call("profiler", "status") or "null", false)
    helpers.write_file("profile-ticks.txt", tostring(e.tick - started), false)
  end
end)
