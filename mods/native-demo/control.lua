-- writes script-output/native-demo.txt: what the native table offers in this game (or that there is none)
local lines = {}
local function say(s) lines[#lines + 1] = s end
local timing  -- (a profiler prints itself only inside a localised string)
local function flush()
  helpers.write_file("native-demo.txt", table.concat(lines, "\n") .. "\n", false)
  if timing then helpers.write_file("native-demo.txt", { "", "10000 x hello.echo: ", timing, "\n" }, true) end
end

local job

local function report()
  if not native then
    say("no native table: plain game")
    return flush()
  end
  say("fse " .. native.version())
  native.log("hello from native-demo's control stage")
  for plugin, fns in pairs(native.plugins()) do say("plugin " .. plugin .. ": " .. table.concat(fns, ", ")) end
  for _, n in ipairs(native.symbols("Inserter::update", 8)) do say("  engine: " .. n) end
  say("hello.echo: " .. tostring(native.call("hello", "echo", "ping")))
  say("hello.reverse: " .. tostring(native.call("hello", "reverse", "factorio")))
  local r, err = native.call("hello", "nope", "")
  say("unknown function: " .. tostring(r) .. ", " .. tostring(err))
  -- the bridge's cost: 10 000 calls of a function that hands its input back
  local prof = helpers.create_profiler()
  for i = 1, 10000 do native.call("hello", "echo", "x") end
  prof.stop()
  timing = prof
  -- Python
  say("py hello: " .. tostring(native.call("py", "native_demo_py:hello", "from lua")))
  say("py hello again: " .. tostring(native.call("py", "native_demo_py:hello", "second")))
  local _, perr = native.call("py", "native_demo_py:fail", "x")
  say("py error: " .. tostring(perr):gsub("\n", " | "))
  -- a slow call on a worker thread, polled from on_tick
  job = native.start("hello", "slow", "40")
  say("started job " .. tostring(job) .. " at tick " .. game.tick)
end

script.on_event(defines.events.on_tick, function(e)
  if e.tick == 1 then report() end
  if job then
    local status, out = native.poll(job)
    if status ~= "pending" then
      say("job " .. job .. " " .. tostring(status) .. " at tick " .. e.tick .. ": " .. tostring(out))
      job = nil
      flush()
    end
  end
  if e.tick == 170 then say("tick 170, done") flush() end
end)
