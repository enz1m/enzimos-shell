enzui_desktop = enzui_desktop or { tiling = false, saved = {} }

local function borrowed(window)
    for _, tag in ipairs(window.tags or {}) do
        if tag:match("^enzremote%-guard%-%x+$") then return true end
    end
    return false
end

local function parked(window)
    for _,tag in ipairs(window.tags or {}) do if tag:match("^enzui%-sidebar%-") then return true end end
    return false
end

local function regular(window)
    return not parked(window) and not borrowed(window) and window.mapped and not window.hidden and not window.pinned
        and window.class ~= "dev.enzui.Dock" and window.class ~= "dev.enzui.Desktop" and window.class ~= "dev.enzui.Launcher" and window.class ~= "dev.enzui.QuickSearch"
        and window.class ~= "dev.enzui.ControlCenter" and window.class ~= "dev.enzui.Sidebar"
        and window.class ~= "dev.enzui.SidebarPane" and window.class ~= "dev.enzui.SidebarBacking" and window.class ~= "dev.enzui.WorkspaceOverview"
        and window.class ~= "dev.enzui.PluginPanel" and window.class ~= "dev.enzui.WindowSpread"
        and window.class ~= "dev.enzui.AssistantPanel" and window.class ~= "dev.enzui.AssistantTrigger"
        and window.class ~= "dev.enzui.NotificationCenter" and window.class ~= "dev.enzui.NotificationToasts"
end

local function checked(dispatcher)
    local result = hl.dispatch(dispatcher)
    if result and result.ok == false then error(result.error or "Window operation failed") end
end

-- Per-desktop mode overrides the default; floating rules preserve dialog behavior.
if enzui_desktop.default == nil then enzui_desktop.default = enzui_desktop.tiling == true end
enzui_desktop.modes = enzui_desktop.modes or {}
enzui_desktop.home = enzui_desktop.home or {}

function enzui_desktop_tiling_at(id)
    if type(id) == "number" and enzui_desktop.modes[id] ~= nil then return enzui_desktop.modes[id] end
    return enzui_desktop.default
end

local function active_id()
    local ws = hl.get_active_workspace and hl.get_active_workspace()
    return ws and ws.id
end

local function sync_rules()
    -- The single rule of the global mode, retired; a reloaded session drops it.
    hl.window_rule({ name = "desktop-floating-default", match = { class = ".*" }, enabled = false, float = true })
    local last = math.max(10, enzui_desktop.rule_last or 0)
    for id in pairs(enzui_desktop.modes) do last = math.max(last, id) end
    for _, ws in ipairs(hl.get_workspaces and hl.get_workspaces() or {}) do
        if type(ws.id) == "number" and ws.id > 0 then last = math.max(last, ws.id) end
    end
    for id = 1, last do
        hl.window_rule({ name = "desktop-floating-" .. id, match = { class = ".*", workspace = tostring(id) },
            enabled = not enzui_desktop_tiling_at(id), float = true })
    end
    -- Special workspaces (the scratchpad) follow the default.
    hl.window_rule({ name = "desktop-floating-special", match = { class = ".*", workspace = "s[true]" },
        enabled = not enzui_desktop.default, float = true })
    enzui_desktop.rule_last = last
    enzui_desktop.tiling = enzui_desktop_tiling_at(active_id())
end

local function workspace_of(window)
    return window.workspace and window.workspace.id
end

-- Restore saved floating geometry when a tiled window returns to floating mode.
local function convert(window, tile)
    local previous = enzui_desktop.saved[window.address]
    if tile then
        if window.floating then
            enzui_desktop.saved[window.address] = { window = window, at = window.at, size = window.size }
            checked(hl.dsp.window.float({ window = window, action = "off" }))
        end
    else
        if not window.floating then checked(hl.dsp.window.float({ window = window, action = "on" })) end
        -- A retained weak window handle prevents address reuse from restoring another client.
        if previous and previous.window.mapped and previous.window.address == window.address then
            checked(hl.dsp.window.resize({ window = window, x = previous.size.x, y = previous.size.y }))
            checked(hl.dsp.window.move({ window = window, x = previous.at.x, y = previous.at.y }))
        end
        enzui_desktop.saved[window.address] = nil
    end
end

-- Convert windows only on desktops whose effective mode changed, unless forced.
function enzui_set_desktop_modes(default, modes, force)
    assert(type(default) == "boolean")
    assert(type(modes) == "table")
    local clean = {}
    for id, mode in pairs(modes) do
        assert(type(id) == "number" and id % 1 == 0 and id > 0 and type(mode) == "boolean", "Invalid desktop mode")
        clean[id] = mode
    end
    local all = hl.get_windows()
    local before = {}
    for _, window in ipairs(all) do
        local id = workspace_of(window)
        if id ~= nil and before[id] == nil then before[id] = enzui_desktop_tiling_at(id) end
    end
    enzui_desktop.default = default
    enzui_desktop.modes = clean
    sync_rules()
    local source, placeholder
    for _, window in ipairs(all) do
        if window.mapped and borrowed(window) then source = window end
        if window.mapped and window.class == "dev.enzui.RemotePlaceholder" then placeholder = window end
    end
    -- Remote loans inherit their placeholder's saved floating geometry.
    if source and placeholder then
        local previous = enzui_desktop.saved[source.address]
        if previous and previous.window.mapped and previous.window.address == source.address
            and not enzui_desktop.saved[placeholder.address] then
            enzui_desktop.saved[placeholder.address] = { window = placeholder, at = previous.at, size = previous.size }
        end
    end
    for _, window in ipairs(all) do
        local id = workspace_of(window)
        local tile = enzui_desktop_tiling_at(id)
        if regular(window) and window.fullscreen == 0 and (force or before[id] ~= tile) then
            convert(window, tile)
        end
    end
    if source and placeholder and enzui_desktop.saved[placeholder.address] and enzui_desktop_tiling_at(workspace_of(placeholder)) then
        local previous = enzui_desktop.saved[placeholder.address]
        enzui_desktop.saved[source.address] = { window = source, at = previous.at, size = previous.size }
    end
    for address, saved in pairs(enzui_desktop.saved) do
        if not saved.window.mapped or saved.window.address ~= address then enzui_desktop.saved[address] = nil end
    end
    return "ok"
end

-- Every desktop at once, forgetting the per-desktop modes.
function enzui_set_tiling(enabled)
    assert(type(enabled) == "boolean")
    return enzui_set_desktop_modes(enabled, {}, true)
end

-- Moved windows adopt the destination mode; special-workspace restore remains caller-owned.
function enzui_desktop_window_moved(window, workspace)
    local from = enzui_desktop.home[window.address]
    local to = workspace and workspace.id
    enzui_desktop.home[window.address] = to
    if type(from) ~= "number" or from <= 0 or type(to) ~= "number" or to <= 0 or from == to then return end
    if not regular(window) or window.fullscreen ~= 0 then return end
    local tile = enzui_desktop_tiling_at(to)
    if tile ~= enzui_desktop_tiling_at(from) then convert(window, tile) end
end

-- Restore hidden windows if the spread overlay disappears or its desktop changes.
enzui_desktop.spread = enzui_desktop.spread or {}

local function set_opacity(window, value)
    hl.dispatch(hl.dsp.window.set_prop({ prop = "opacity", value = value, window = window }))
    hl.dispatch(hl.dsp.window.set_prop({ prop = "opacity_inactive", value = value, window = window }))
end

function enzui_spread_hide(addresses)
    assert(type(addresses) == "table", "Invalid window list")
    local wanted = {}
    for _, address in ipairs(addresses) do
        assert(type(address) == "string" and address:match("^0x%x+$"), "Invalid window")
        wanted[address] = true
    end
    local desktop = active_id()
    -- Floating and tiled desktops alike: windows are only made transparent
    -- while their pictures stand in for them, so tiles keep their places.
    for _, window in ipairs(hl.get_windows()) do
        if wanted[window.address] and regular(window) and workspace_of(window) == desktop then
            enzui_desktop.spread[window.address] = window
            -- At once: their pictures cover them, and a fade would be
            -- composited under the spread's own motion.
            hl.dispatch(hl.dsp.window.set_prop({ prop = "no_anim", value = "1", window = window }))
            set_opacity(window, "0")
        end
    end
    for _, window in ipairs(hl.get_windows()) do
        if window.mapped and (window.class == "dev.enzui.Desktop" or window.class == "dev.enzui.AssistantTrigger") then
            hl.dispatch(hl.dsp.window.alter_zorder({ mode = "top", window = window }))
        end
    end
    return "ok"
end

function enzui_spread_restore(focus)
    assert(focus == nil or type(focus) == "string" and focus:match("^0x%x+$"), "Invalid window")
    for address, window in pairs(enzui_desktop.spread) do
        -- A retained weak window handle: a reused address is another client.
        if window.mapped and window.address == address then
            hl.dispatch(hl.dsp.window.set_prop({ prop = "no_anim", value = "unset", window = window }))
            set_opacity(window, "1")
        end
    end
    enzui_desktop.spread = {}
    local target
    for _, window in ipairs(focus and hl.get_windows() or {}) do
        if window.address == focus and regular(window) then target = window end
    end
    if target then
        -- The pointer stays where the user clicked.
        local no_warps = hl.get_config("cursor.no_warps")
        hl.config({ cursor = { no_warps = true } })
        local ok, error = pcall(function()
            checked(hl.dsp.window.alter_zorder({ mode = "top", window = target }))
            checked(hl.dsp.focus({ window = target }))
        end)
        hl.config({ cursor = { no_warps = no_warps } })
        assert(ok, error)
    end
    return "ok"
end

local function spread_gone()
    if next(enzui_desktop.spread) then enzui_spread_restore(nil) end
end

if hl.on and not enzui_desktop.spread_bindings then
    enzui_desktop.spread_bindings = {
        hl.on("window.close", function(window)
            if window and window.class == "dev.enzui.WindowSpread" then spread_gone() end
        end),
        hl.on("workspace.active", spread_gone),
    }
end

-- A new window on a floating desktop that would open exactly over another
-- one steps down and right until it does not, so stacked windows stay apart;
-- past the monitor's usable area the steps start again from its corner.
local CASCADE = 40
function enzui_desktop_cascade(window)
    if not (window and regular(window) and window.floating and window.at and window.size) then return end
    local id = workspace_of(window)
    if id == nil or enzui_desktop_tiling_at(id) then return end
    local others = {}
    for _, other in ipairs(hl.get_windows()) do
        if other.address ~= window.address and regular(other) and other.floating and workspace_of(other) == id and other.at then
            others[#others + 1] = other
        end
    end
    -- Over another window means centred on (nearly) the same point: windows
    -- of different sizes both opened centred share a centre, not a corner.
    local function taken(x, y)
        local cx, cy = x + window.size.x / 2, y + window.size.y / 2
        for _, other in ipairs(others) do
            local ox, oy = other.at.x + (other.size and other.size.x or 0) / 2, other.at.y + (other.size and other.size.y or 0) / 2
            if math.abs(ox - cx) < 36 and math.abs(oy - cy) < 36 then return true end
        end
        return false
    end
    local x, y = window.at.x, window.at.y
    if not taken(x, y) then return end
    local monitor = window.monitor
    local scale = monitor and monitor.scale or 1
    local reserved = monitor and monitor.reserved or {}
    local left = (monitor and monitor.x or 0) + (reserved.left or 0)
    local top = (monitor and monitor.y or 0) + (reserved.top or 0)
    local right = (monitor and monitor.x or 0) + math.floor((monitor and monitor.width or 1e6) / scale + 0.5) - (reserved.right or 0)
    local bottom = (monitor and monitor.y or 0) + math.floor((monitor and monitor.height or 1e6) / scale + 0.5) - (reserved.bottom or 0)
    for _ = 1, 64 do
        x, y = x + CASCADE, y + CASCADE
        if x + window.size.x > right or y + window.size.y > bottom then
            x, y = left + CASCADE / 2, top + CASCADE / 2
        end
        if not taken(x, y) then break end
    end
    checked(hl.dsp.window.move({ window = window, x = math.floor(x), y = math.floor(y) }))
end

if hl.on and not enzui_desktop.bindings then
    enzui_desktop.bindings = {
        hl.on("window.open", function(window)
            enzui_desktop.home[window.address] = workspace_of(window)
            enzui_desktop_cascade(window)
        end),
        hl.on("window.destroy", function(window) if window then enzui_desktop.home[window.address] = nil end end),
        hl.on("window.move_to_workspace", function(window, workspace) enzui_desktop_window_moved(window, workspace) end),
        hl.on("workspace.active", function()
            enzui_desktop.tiling = enzui_desktop_tiling_at(active_id())
            if enzui_sidebar_wake then enzui_sidebar_wake() end
        end),
        hl.on("workspace.created", function(workspace)
            if workspace and type(workspace.id) == "number" and workspace.id > (enzui_desktop.rule_last or 0) then sync_rules() end
        end),
    }
end
sync_rules()
