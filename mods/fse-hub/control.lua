-- fse hub: one button (the mod-gui bar, top left) for everything fse adds. In the main menu the same hub is
-- the "fse hub" button the web plugin draws over the game window's corner.
--
-- The window has tabs: Hub (loader, engine profiler, agents), Mods (the mod manager: added / updated dates, load
-- order, portal updates) and Startup (what took the game how long to start). Mods and Startup ask Python
-- (fse_tools.mods / .startup) on a worker thread (native.start), so a tick never waits; the Dashboard button opens
-- the web pages in a panel over the game (web.open).

local mod_gui = require("mod-gui")
local events = require("__fse-std__/events")
local input = require("__fse-std__/input")
local window = require("__fse-std__/window")

local NAME = "fse_hub"
local PAGE = 100

local function loaded()
  return type(native) == "table" and type(native.plugins) == "function" and type(native.call) == "function"
    and not script.level.is_simulation
end

local function plugins()
  local list = {}
  for p in pairs(loaded() and native.plugins() or {}) do list[#list + 1] = p end
  table.sort(list)
  return list
end

local function has(plugin)
  if not loaded() then return false end
  local ok, list = pcall(native.plugins)
  return ok and type(list) == "table" and list[plugin] ~= nil
end

local function find(el, name)
  if not (el and el.valid) then return nil end
  if el.name == name then return el end
  for _, c in pairs(el.children) do
    local f = find(c, name)
    if f then return f end
  end
end

-- the button only with the loader (a save opened without it keeps no button)
local function sync_button(player)
  local flow = mod_gui.get_button_flow(player)
  local b = flow.fse_hub_button
  if loaded() and not b then
    flow.add({ type = "sprite-button", name = "fse_hub_button", sprite = "virtual-signal/signal-F",
      style = mod_gui.button_style, tooltip = "fse hub: mods, startup, engine profiler, agents" })
  elseif not loaded() and b then
    b.destroy()
  end
end

local function web_url(path)
  if not has("web") then return nil end
  local base = native.call("web", "url")
  if not base then return nil end
  -- (base is ".../?token=...": a page goes before the query)
  return (base:gsub("/%?", "/" .. (path or "") .. "?", 1))
end

-- a web page in the panel over the game (older web plugins: the browser)
local function open_page(path)
  if has("web") and native.call("web", "open", path) then return end
  local url = web_url(path)
  if url and has("std") then native.call("std", "open", url) end
end

---------------------------------------------------------------------------------------------------------------------
-- data from Python, on a worker thread

local JOBS = {
  mods = "fse_tools.mods:listing",
  portal = "fse_tools.mods:portal",
  startup = "fse_tools.startup:report",
}
local jobs = {}   -- kind -> job id
local data = {}   -- kind -> the answer as a table, or { error = "..." }
local views = {}  -- player_index -> what each tab shows (sort, search, page)

local function fetch(kind)
  if jobs[kind] or not has("py") then return end
  local id, err = native.start("py", JOBS[kind], "")
  if id then jobs[kind] = id else data[kind] = { error = err or "could not start" } end
end

local function view(player_index)
  local v = views[player_index]
  if not v then
    v = { mods = { sort = "load", dir = 1, search = "", filter = 1, page = 1 },
          startup = { sort = "total_est", dir = -1, search = "", page = 1 } }
    views[player_index] = v
  end
  return v
end

---------------------------------------------------------------------------------------------------------------------
-- formatting (game Lua has no os.date)

local function day(t)
  if not t then return "" end
  local z = math.floor(t / 86400) + 719468
  local era = math.floor(z / 146097)
  local doe = z - era * 146097
  local yoe = math.floor((doe - math.floor(doe / 1460) + math.floor(doe / 36524) - math.floor(doe / 146096)) / 365)
  local doy = doe - (365 * yoe + math.floor(yoe / 4) - math.floor(yoe / 100))
  local mp = math.floor((5 * doy + 2) / 153)
  local d = doy - math.floor((153 * mp + 2) / 5) + 1
  local m = mp < 10 and mp + 3 or mp - 9
  local y = yoe + era * 400 + (m <= 2 and 1 or 0)
  return string.format("%04d-%02d-%02d", y, m, d)
end

local function size(b)
  if not b then return "" end
  if b >= 1048576 then return string.format("%.1f MB", b / 1048576) end
  return string.format("%d kB", math.ceil(b / 1024))
end

local function secs(s)
  if not s then return "?" end
  if s >= 60 then return string.format("%d min %d s", math.floor(s / 60), math.floor(s % 60 + 0.5)) end
  return string.format("%.1f s", s)
end

local function vkey(v)
  local k = {}
  for n in tostring(v):gmatch("%d+") do k[#k + 1] = tonumber(n) end
  return k
end

local function newer(a, b)
  local x, y = vkey(a), vkey(b)
  for i = 1, math.max(#x, #y) do
    if (x[i] or 0) ~= (y[i] or 0) then return (x[i] or 0) > (y[i] or 0) end
  end
  return false
end

---------------------------------------------------------------------------------------------------------------------
-- a sortable, searchable, paged table: headers are clickable labels

local function grid(parent, tab, cols, rows, state)
  local key = state.sort
  local col
  for _, c in ipairs(cols) do if c.key == key then col = c end end
  local sorter = col and col.sort or function(r) return r[key] end
  table.sort(rows, function(a, b)
    local x, y = sorter(a), sorter(b)
    if x == y then return (a.name or "") < (b.name or "") end
    if x == nil then return false end
    if y == nil then return true end
    if state.dir > 0 then return x < y else return x > y end
  end)
  local pages = math.max(1, math.ceil(#rows / PAGE))
  state.page = math.min(state.page, pages)
  local first = (state.page - 1) * PAGE + 1
  local last = math.min(#rows, first + PAGE - 1)

  local pager = parent.add({ type = "flow", direction = "horizontal" })
  pager.style.vertical_align = "center"
  pager.add({ type = "button", caption = "<", style = "tool_button", tags = { fhub_page = tab, d = -1 },
    enabled = state.page > 1 })
  pager.add({ type = "label", caption = #rows == 0 and "none" or string.format("%d-%d of %d", first, last, #rows) })
  pager.add({ type = "button", caption = ">", style = "tool_button", tags = { fhub_page = tab, d = 1 },
    enabled = state.page < pages })

  local scroll = parent.add({ type = "scroll-pane", horizontal_scroll_policy = "never" })
  scroll.style.vertically_stretchable = true
  scroll.style.horizontally_stretchable = true
  local t = scroll.add({ type = "table", column_count = #cols, draw_horizontal_line_after_headers = true })
  t.style.horizontal_spacing = 12
  for _, c in ipairs(cols) do
    local arrow = c.key == key and (state.dir > 0 and " ▲" or " ▼") or ""
    t.add({ type = "label", caption = c.caption .. arrow, style = "caption_label", tooltip = "Sort",
      tags = { fhub_sort = tab, key = c.key, desc = c.desc or false } })
  end
  for i = first, last do
    local r = rows[i]
    for _, c in ipairs(cols) do
      local cell = c.cell(r, i)
      if type(cell) == "table" and cell.bar then
        local bar = t.add({ type = "progressbar", value = math.min(1, math.max(0, cell.bar)) })
        bar.style.width = 120
      else
        local label = t.add({ type = "label", caption = type(cell) == "table" and cell[1] or tostring(cell or ""),
          tooltip = type(cell) == "table" and cell.tooltip or nil })
        if c.width then label.style.maximal_width = c.width end
        if r.enabled == false then label.style.font_color = { 0.55, 0.55, 0.55 } end
      end
    end
  end
end

local function message(parent, text)
  parent.add({ type = "label", caption = text }).style.single_line = false
end

---------------------------------------------------------------------------------------------------------------------
-- Mods tab

local FILTERS = { "All mods", "Enabled", "Disabled", "Added in the last 7 days", "Update on the portal" }

local function render_mods(player)
  local frame = player.gui.screen[NAME]
  local body = frame and find(frame, "fhub_mods_body")
  if not body then return end
  body.clear()
  local d = data.mods
  if not d then
    fetch("mods")
    return message(body, "Reading the mods folder...")
  end
  if d.error then return message(body, "Could not read the mods: " .. tostring(d.error)) end
  local state = view(player.index).mods
  local portal = data.portal and not data.portal.error and data.portal or (data.portal and data.portal.cached) or {}
  local now = input.now() and input.now() / 1000  -- (ms)
  local search = state.search:lower()
  local rows = {}
  for _, m in ipairs(d.mods or {}) do
    local p = portal[m.name]
    m.update = p and p.version and newer(p.version, m.version) and p.version or nil
    local hay = (m.name .. " " .. (m.title or "") .. " " .. (m.author or "")):lower()
    local ok = search == "" or hay:find(search, 1, true)
    local f = state.filter
    ok = ok and (f == 1 or (f == 2 and m.enabled) or (f == 3 and not m.enabled)
      or (f == 4 and now and m.added and now - m.added < 7 * 86400) or (f == 5 and m.update ~= nil))
    if ok then rows[#rows + 1] = m end
  end
  local info = body.add({ type = "label", caption = string.format("%d mods, %d enabled · load order from the game's log%s",
    d.total or 0, d.enabled or 0, data.portal and data.portal.error and (" · portal: " .. data.portal.error) or "") })
  info.style.font_color = { 0.7, 0.7, 0.7 }
  grid(body, "mods", {
    { key = "load", caption = "#", cell = function(m) return m.load or "" end },
    { key = "name", caption = "Mod", width = 260, sort = function(m) return (m.title or m.name):lower() end,
      cell = function(m)
        return { m.title or m.name, tooltip = m.name .. " " .. m.version .. (m.author ~= "" and (" by " .. m.author) or "")
          .. "\n" .. (m.description or "") .. "\n" .. m.file .. (m.enabled and "" or "\n(disabled)") }
      end },
    { key = "version", caption = "Version", sort = function(m) return m.version end, cell = function(m) return m.version end },
    { key = "added", caption = "Added", desc = true, cell = function(m) return day(m.added) end },
    { key = "updated", caption = "Updated", desc = true, cell = function(m) return day(m.updated) end },
    { key = "size", caption = "Size", desc = true, cell = function(m) return size(m.size) end },
    { key = "update", caption = "Portal", sort = function(m) return m.update and 0 or 1 end,
      cell = function(m)
        if m.update then return { "[color=255,200,100]" .. m.update .. "[/color]", tooltip = "a newer version is on the mod portal" } end
        return portal[m.name] and "latest" or ""
      end },
  }, rows, state)
end

---------------------------------------------------------------------------------------------------------------------
-- Startup tab

local function render_startup(player)
  local frame = player.gui.screen[NAME]
  local body = frame and find(frame, "fhub_startup_body")
  if not body then return end
  body.clear()
  local d = data.startup
  if not d then
    fetch("startup")
    return message(body, "Reading the game's log...")
  end
  if d.error then return message(body, "Could not read the startup: " .. tostring(d.error)) end
  body.add({ type = "label", caption = "Started in " .. secs(d.total), style = "caption_label" })
  -- (two phases a row: the mods below get the room)
  local phases = body.add({ type = "table", column_count = 6 })
  phases.style.horizontal_spacing = 10
  for _, p in ipairs(d.phases or {}) do
    phases.add({ type = "label", caption = p.name, tooltip = p.what ~= "" and p.what or nil })
    phases.add({ type = "progressbar", value = d.total and d.total > 0 and math.min(1, (p.seconds or 0) / d.total) or 0 }).style.width = 110
    phases.add({ type = "label", caption = secs(p.seconds) }).style.minimal_width = 70
  end
  body.add({ type = "line" })
  local state = view(player.index).startup
  local search = state.search:lower()
  local rows, top = {}, 0
  for _, m in ipairs(d.mods or {}) do
    top = math.max(top, m.total_est or 0)
    if search == "" or m.name:lower():find(search, 1, true) then rows[#rows + 1] = m end
  end
  local stage_tip = function(m)
    local parts = {}
    for stage, s in pairs(m.stages or {}) do parts[#parts + 1] = stage .. " " .. string.format("%.2f s", s) end
    table.sort(parts)
    return table.concat(parts, "\n")
  end
  grid(body, "startup", {
    { key = "name", caption = "Mod", width = 240, sort = function(m) return m.name:lower() end,
      cell = function(m) return { m.name, tooltip = stage_tip(m) } end },
    { key = "lua", caption = "Lua", desc = true, cell = function(m) return string.format("%.2f s", m.lua or 0) end },
    { key = "sprites_est", caption = "Sprites ≈", desc = true, cell = function(m) return string.format("%.2f s", m.sprites_est or 0) end },
    { key = "sounds_est", caption = "Sounds ≈", desc = true, cell = function(m) return string.format("%.2f s", m.sounds_est or 0) end },
    { key = "total_est", caption = "Total ≈", desc = true, cell = function(m) return string.format("%.2f s", m.total_est or 0) end },
    { key = "bar", caption = "", sort = function(m) return m.total_est end,
      cell = function(m) return { bar = top > 0 and (m.total_est or 0) / top or 0 } end },
  }, rows, state)
  local note = body.add({ type = "label", caption = "Lua is measured from the log; sprites and sounds are split by each mod's image and sound bytes (estimates)." })
  note.style.font_color = { 0.7, 0.7, 0.7 }
  note.style.single_line = false
end

---------------------------------------------------------------------------------------------------------------------
-- Hub tab

local function profile_rows(table_el)
  table_el.clear()
  for _, h in ipairs({ "function", "ms/tick", "calls/tick" }) do
    table_el.add({ type = "label", caption = h, style = "caption_label" })
  end
  local p = has("profiler") and native.call("profiler", "profile", "hub")
  p = p and helpers.json_to_table(p)
  if not (p and p.active) then
    table_el.add({ type = "label", caption = "stopped (no timing cost)" })
    table_el.add({ type = "empty-widget" })
    table_el.add({ type = "empty-widget" })
    return
  end
  local n = 0
  for _, f in ipairs(p.functions or {}) do
    if f.calls > 0 and n < 10 then
      n = n + 1
      table_el.add({ type = "label", caption = f.name })
      table_el.add({ type = "label", caption = string.format("%.3f", f.ms_per_tick) })
      table_el.add({ type = "label", caption = string.format("%.1f", f.calls_per_tick) })
    end
  end
end

local function hub_tab(content)
  local build = native.build and native.build() or {}
  content.add({ type = "label", caption = "fse " .. native.version() .. " · build " .. tostring(build.build or "?"):sub(1, 8)
    .. " · plugins: " .. table.concat(plugins(), ", ") }).style.single_line = false
  local links = content.add({ type = "flow", direction = "horizontal" })
  local bp = remote.interfaces["bpgen"] and "bpgen" or remote.interfaces["bpgen-companion"] and "bpgen-companion"
  if bp and remote.interfaces[bp].open_window then  -- (bpgen's mod; "bpgen-companion" before 0.6)
    links.add({ type = "button", caption = "bpgen", tags = { fhub = "bpgen" }, tooltip = "Plan a production line, previewed by the game" })
  end
  if has("web") then
    links.add({ type = "button", caption = "Dashboard", tags = { fhub = "dashboard" }, tooltip = "The web dashboard, in a panel over the game" })
    links.add({ type = "button", caption = "Copy link", tags = { fhub = "copy" }, tooltip = "The dashboard's address with its token (for a browser or an agent)" })
  end
  content.add({ type = "line" })
  local prow = content.add({ type = "flow", direction = "horizontal" })
  prow.add({ type = "label", caption = "Engine profile", style = "caption_label" })
  if has("profiler") then
    prow.add({ type = "button", caption = "Start", tags = { fhub = "start" }, style = "green_button" })
    prow.add({ type = "button", caption = "Stop", tags = { fhub = "stop" } })
  end
  local scroll = content.add({ type = "scroll-pane", name = "fhub_scroll" })
  scroll.style.vertically_stretchable = true
  profile_rows(scroll.add({ type = "table", name = "fhub_profile", column_count = 3 }))
  if remote.interfaces.agent then
    content.add({ type = "line" })
    local names = {}
    for n in pairs(remote.call("agent", "list") or {}) do names[#names + 1] = n end
    content.add({ type = "label", caption = "Agents: " .. (#names > 0 and table.concat(names, ", ") or "none (spawn them over the web API)") })
  end
end

---------------------------------------------------------------------------------------------------------------------

local TABS = { "hub", "mods", "startup" }

local function render(player, tab)
  if tab == "mods" then render_mods(player) elseif tab == "startup" then render_startup(player) end
end

local function open(player, tab)
  local frame, content = window.create(player, { name = NAME, title = "fse hub", width = 960, height = 680,
    min_width = 420, min_height = 260 })
  if not loaded() then
    content.add({ type = "label", caption = "Start the game with fse.exe to use fse." })
    return frame
  end
  local v = view(player.index)
  v.tab = tab or v.tab or "hub"
  local tp = content.add({ type = "tabbed-pane", name = "fhub_tabs" })
  tp.style.vertically_stretchable = true
  tp.style.horizontally_stretchable = true
  for i, name in ipairs(TABS) do
    local t = tp.add({ type = "tab", caption = ({ hub = "Hub", mods = "Mods", startup = "Startup" })[name], tags = { fhub_tab = name } })
    local body = tp.add({ type = "flow", direction = "vertical" })
    body.style.vertically_stretchable = true
    body.style.horizontally_stretchable = true
    tp.add_tab(t, body)
    if name == "hub" then
      hub_tab(body)
    else
      local controls = body.add({ type = "flow", direction = "horizontal" })
      controls.style.vertical_align = "center"
      controls.add({ type = "textfield", text = v[name].search, tags = { fhub_search = name }, tooltip = "Search" })
      if name == "mods" then
        controls.add({ type = "drop-down", items = FILTERS, selected_index = v.mods.filter, tags = { fhub_filter = "mods" } })
        controls.add({ type = "button", caption = "Check the portal", tags = { fhub = "portal" },
          tooltip = "Asks the mod portal which mods have a newer version" })
      end
      controls.add({ type = "button", caption = "Reload", tags = { fhub = "reload", tab = name } })
      local inner = body.add({ type = "flow", direction = "vertical", name = "fhub_" .. name .. "_body" })
      inner.style.vertically_stretchable = true
      inner.style.horizontally_stretchable = true
    end
    if name == v.tab then tp.selected_tab_index = i end
  end
  render(player, v.tab)
  return frame
end

local function rerender(kind)
  for _, player in pairs(game.connected_players) do
    if player.gui.screen[NAME] then
      if kind == "startup" then render_startup(player) else render_mods(player) end
    end
  end
end

local function click(e)
  local el = e.element
  if not (el and el.valid) then return end
  local player = game.get_player(e.player_index)
  if el.name == "fse_hub_button" then
    if player.gui.screen[NAME] then player.gui.screen[NAME].destroy() else open(player) end
    return
  end
  local tags = el.tags
  local v = view(player.index)
  if tags.fhub_sort then
    local s = v[tags.fhub_sort]
    if s.sort == tags.key then s.dir = -s.dir else s.sort, s.dir = tags.key, tags.desc and -1 or 1 end
    s.page = 1
    return render(player, tags.fhub_sort)
  elseif tags.fhub_page then
    local s = v[tags.fhub_page]
    s.page = math.max(1, s.page + tags.d)
    return render(player, tags.fhub_page)
  end
  local action = tags.fhub
  if not action then return end
  if action == "bpgen" then
    player.gui.screen[NAME].destroy()
    remote.call(remote.interfaces["bpgen"] and "bpgen" or "bpgen-companion", "open_window", player.index)
  elseif action == "dashboard" then
    open_page("")
  elseif action == "copy" then
    local url = web_url("")
    if url and input.clipboard_set(url) then player.print("[fse] dashboard link copied") end
  elseif action == "portal" then
    data.portal = nil
    fetch("portal")
    player.print("[fse] asking the mod portal...")
  elseif action == "reload" then
    data[tags.tab] = nil
    render(player, tags.tab)
  elseif action == "start" or action == "stop" then
    native.call("profiler", action)
    native.call("profiler", "profile", "hub")  -- (a fresh baseline)
    local p = find(player.gui.screen[NAME], "fhub_profile")
    if p then profile_rows(p) end
  end
end

local function poll()
  for kind, id in pairs(jobs) do
    local status, out = native.poll(id)
    if status ~= "pending" then
      jobs[kind] = nil
      if status == "done" then
        data[kind] = helpers.json_to_table(out) or { error = "unreadable answer" }
      else
        data[kind] = { error = tostring(out) }
      end
      rerender(kind == "portal" and "mods" or kind)
    end
  end
end

local function refresh()
  if not has("profiler") then return end
  for _, player in pairs(game.connected_players) do
    local frame = player.gui.screen[NAME]
    local v = views[player.index]
    if frame and (not v or v.tab == "hub") then
      local p = find(frame, "fhub_profile")
      if p then profile_rows(p) end
    end
  end
end

local function sync_all()
  for _, player in pairs(game.players) do sync_button(player) end
end

local safe = require("__fse-std__/safe")
script.on_init(safe.guard("fse-hub", sync_all))
script.on_configuration_changed(safe.guard("fse-hub", sync_all))
script.on_nth_tick(60, safe.guard("fse-hub", refresh))
script.on_nth_tick(10, safe.guard("fse-hub", function() if next(jobs) then poll() end end))

-- for tests
remote.add_interface("fse-hub", { open = function(pi, tab) open(game.get_player(pi), tab) end,
  url = function(path) return web_url(path) end,
  loaded = function(kind) return data[kind] ~= nil and (data[kind].error or true) end })

events.register({ input.handlers, window.handlers, {
  [defines.events.on_gui_click] = click,
  [defines.events.on_gui_selected_tab_changed] = function(e)
    local tp = e.element
    if not (tp and tp.valid and tp.name == "fhub_tabs") then return end
    local tab = tp.tabs[tp.selected_tab_index].tab.tags.fhub_tab
    view(e.player_index).tab = tab
    render(game.get_player(e.player_index), tab)
  end,
  [defines.events.on_gui_text_changed] = function(e)
    local el = e.element
    local tab = el and el.valid and el.tags.fhub_search
    if not tab then return end
    local s = view(e.player_index)[tab]
    s.search, s.page = el.text, 1
    render(game.get_player(e.player_index), tab)
  end,
  [defines.events.on_gui_selection_state_changed] = function(e)
    local el = e.element
    if not (el and el.valid and el.tags.fhub_filter) then return end
    local s = view(e.player_index).mods
    s.filter, s.page = el.selected_index, 1
    render_mods(game.get_player(e.player_index))
  end,
  [defines.events.on_player_created] = function(e) sync_button(game.get_player(e.player_index)) end,
  [defines.events.on_player_joined_game] = function(e) sync_button(game.get_player(e.player_index)) end,
} })
