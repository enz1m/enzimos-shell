-- Drag-to-maximize preview for titlebar and compositor-driven moves.
-- Hyprland has no window-moved event; sample positions only while the pointer button is held.

local shell_classes = {
    ["dev.enzui.Desktop"] = true, ["dev.enzui.Launcher"] = true, ["dev.enzui.QuickSearch"] = true,
    ["dev.enzui.ControlCenter"] = true, ["dev.enzui.WorkspaceOverview"] = true,
    ["dev.enzui.Sidebar"] = true, ["dev.enzui.SidebarPane"] = true, ["dev.enzui.SidebarBacking"] = true,
    ["dev.enzui.PluginPanel"] = true, ["dev.enzui.NotificationCenter"] = true,
    ["dev.enzui.AssistantPanel"] = true, ["dev.enzui.AssistantTrigger"] = true,
    ["dev.enzui.NotificationToasts"] = true, ["dev.enzui.RemotePlaceholder"] = true,
}

-- Arm within this depth below the reserved top band.
local TRIGGER_DEPTH = 12

-- A press without pointer travel must not become a drag-to-maximize gesture.
local RESTORE_THRESHOLD = 6

enzui_drag_maximize = enzui_drag_maximize or {
    pipe = nil, command = nil, armed = false, window = nil,
    timer = nil, target = nil,
}

local function eligible(window)
    return window and window.mapped and not window.hidden and window.floating
        and not window.pinned and window.fullscreen == 0
        and not shell_classes[window.class or ""]
end

-- Exclude shell-owned, pinned and hidden windows from restore gestures.
local function restorable(window)
    return window and window.mapped and not window.hidden and window.floating
        and not window.pinned and window.fullscreen == 1
        and not shell_classes[window.class or ""]
end

local function geometry_key(monitor)
    local reserved = monitor.reserved or {}
    return string.format("%s:%d:%d:%d:%d:%d:%d:%d:%d",
        tostring(monitor.name), monitor.x, monitor.y, monitor.width, monitor.height,
        reserved.top or 0, reserved.bottom or 0, reserved.left or 0, reserved.right or 0)
end

-- Estimate until a settled maximize provides exact geometry; immediate dispatch readback is
-- stale.
local function estimated_rect(monitor)
    local reserved = monitor.reserved
    if type(reserved) ~= "table" then return nil end
    local border = 0
    local ok, value = pcall(hl.get_config, "general:border_size")
    if ok and type(value) == "number" and value >= 0 and value <= 64 then border = value end
    local x = monitor.x + reserved.left + border
    local y = monitor.y + reserved.top + border
    local width = enzui_output_w(monitor) - reserved.left - reserved.right - border * 2
    local height = enzui_output_h(monitor) - reserved.top - reserved.bottom - border * 2
    if width < 1 or height < 1 then return nil end
    return { x = math.floor(x), y = math.floor(y), width = math.floor(width), height = math.floor(height) }
end

local function at_press(window, point)
    local at, size = window and window.at, window and window.size
    return at and size and point and point.x >= at.x and point.y >= at.y
        and point.x < at.x + size.x and point.y < at.y + size.y
end

local function pressed_candidates(point)
    local found = {}
    for _,window in ipairs(hl.get_windows()) do
        if (eligible(window) or restorable(window)) and at_press(window, point) then
            found[#found+1] = {window=window, at={x=window.at.x,y=window.at.y}}
        end
    end
    return found
end

local function target_rect(window)
    local monitor = window.monitor
    if not monitor then return nil end
    local cached = enzui_drag_maximize.target
    if cached and cached.key == geometry_key(monitor) then return cached end
    return estimated_rect(monitor)
end

local function learn(window)
    if not (window and window.mapped and window.monitor) then return end
    local at, size = window.at, window.size
    if not (at and size) or size.x < 1 or size.y < 1 then return end
    if window.fullscreen ~= 1 then return end
    local rect = {
        key = geometry_key(window.monitor),
        x = math.floor(at.x), y = math.floor(at.y),
        width = math.floor(size.x), height = math.floor(size.y),
    }
    enzui_drag_maximize.target = rect
end

local function send(line)
    local state = enzui_drag_maximize
    for _ = 1, 2 do
        if not state.pipe then
            if not state.command then return false end
            -- Detach the preview so closing its pipe cannot block the compositor waiting for
            -- its own client.
            local ok, pipe = pcall(io.popen, "{ " .. state.command .. "\n} 0<&0 &", "w")
            if not ok or not pipe then return false end
            state.pipe = pipe
        end
        local ok = pcall(function()
            local written, write_error = state.pipe:write(line .. "\n")
            if not written then error(write_error or "Preview write failed") end
            local flushed, flush_error = state.pipe:flush()
            if not flushed then error(flush_error or "Preview flush failed") end
        end)
        if ok then return true end
        pcall(function() state.pipe:close() end)
        state.pipe = nil
    end
    return false
end

local function hide_preview()
    local state = enzui_drag_maximize
    if state.preview then send("hide") end
    state.preview = false
    state.armed = false
end

enzui_drag_maximize_accent = enzui_drag_maximize_accent or "4a90d9"
enzui_drag_maximize_radius = enzui_drag_maximize_radius or 12

local function window_travel(window, start)
    local at = window and window.at
    return at and start and math.max(math.abs(at.x - start.x), math.abs(at.y - start.y)) or 0
end

local function moved_target(state)
    local chosen
    for _,candidate in ipairs(state.candidates or {}) do
        local window = candidate.window
        if window and window.mapped and (eligible(window) or restorable(window))
            and window_travel(window, candidate.at) > RESTORE_THRESHOLD then
            if chosen then return nil end
            chosen = candidate
        end
    end
    return chosen
end

local function pointer_travel(press)
    local pointer = hl.get_cursor_pos()
    if not press or not pointer then return nil end
    return math.max(math.abs(pointer.x - press.x), math.abs(pointer.y - press.y))
end

local function in_band(window)
    local monitor = window.monitor
    local pointer = hl.get_cursor_pos()
    if not (monitor and monitor.reserved and pointer) then return false end
    local floor = monitor.y + monitor.reserved.top
    return pointer.y >= monitor.y and pointer.y < floor + TRIGGER_DEPTH
end

local function window_rect(window)
    if not (window.at and window.size) then return nil end
    return { x = window.at.x, y = window.at.y,
        width = window.size.x, height = window.size.y }
end

local function sample()
    local state = enzui_drag_maximize
    local target = moved_target(state)
    local window = target and target.window
    if state.window ~= window then hide_preview() end
    state.window = window
    state.start_at = target and target.at
    state.rect = window and target_rect(window)
    if not (window and state.rect) then
        hide_preview()
        return
    end
    -- The pointer, not the window top, determines whether the band is armed.
    local moved = pointer_travel(state.press_pointer)
    local inside = in_band(window) and moved and moved > RESTORE_THRESHOLD
    state.armed = inside
    if not inside and state.preview then hide_preview() end
    local from, target = window_rect(window), state.rect
    if inside and not state.preview and from and target and window.fullscreen == 0 then
        state.preview = send(string.format("show %d %d %d %d %d %d %d %d %s %d",
            math.floor(from.x), math.floor(from.y),
            math.floor(from.width), math.floor(from.height),
            target.x, target.y, target.width, target.height,
            enzui_drag_maximize_accent, enzui_drag_maximize_radius))
    end
end

local function stop_timer()
    local state = enzui_drag_maximize
    if state.timer then
        state.timer:set_enabled(false)
        state.timer = nil
    end
end

local function press()
    local state = enzui_drag_maximize
    -- A fresh grab invalidates delayed maximize work from the previous release.
    state.generation = (state.generation or 0) + 1
    local pointer = hl.get_cursor_pos()
    stop_timer()
    hide_preview()
    state.window = nil
    state.press_pointer = nil
    state.start_at = nil
    state.candidates = pressed_candidates(pointer)
    if #state.candidates == 0 then return end
    state.rect = nil
    state.press_pointer = pointer
    state.timer = hl.timer(function()
        if not enzui_drag_maximize.press_pointer then return end
        sample()
    end, { timeout = 16, type = "repeat" })
end

local function release()
    local state = enzui_drag_maximize
    stop_timer()
    local target = moved_target(state)
    local window = target and target.window
    local moved = pointer_travel(state.press_pointer)
    local armed = window and target_rect(window) and in_band(window)
        and moved and moved > RESTORE_THRESHOLD
    state.window = nil
    state.press_pointer = nil
    state.start_at = nil
    state.candidates = nil
    hide_preview()
    if not (armed and window.mapped and eligible(window)) then return end
    local ticket = state.generation
    -- Defer maximize until compositor drag-end completes, preserving the saved floating
    -- geometry.
    hl.timer(function()
        local live = enzui_drag_maximize
        if live.generation ~= ticket then return end
        if not (window and window.mapped) then return end
        if not eligible(window) then return end
        hl.dispatch(hl.dsp.window.fullscreen({ window = window, mode = "maximized", action = "set" }))
        -- Read settled geometry for the next preview rather than trusting dispatch-time
        -- dimensions.
        hl.timer(function() learn(window) end, { timeout = 150, type = "oneshot" })
    end, { timeout = 50, type = "oneshot" })
end

function enzui_drag_maximize_install(command)
    assert(type(command) == "string" and #command > 0 and #command <= 8192, "Invalid preview command")
    local state = enzui_drag_maximize
    if state.pipe and state.command ~= command then
        state.retired_pipes = state.retired_pipes or {}
        assert(#state.retired_pipes < 4, "Too many preview output changes")
        pcall(function() state.pipe:write("hide\n");state.pipe:flush() end)
        state.retired_pipes[#state.retired_pipes + 1] = state.pipe
        state.pipe = nil
    end
    state.command = command
    -- Hyprland's zero threshold starts restore on press rather than motion.
    if hl.get_config("binds:drag_threshold") == 0 then
        hl.config({ binds = { drag_threshold = RESTORE_THRESHOLD } })
    end
    stop_timer()
    state.window = nil
    state.armed = false
    state.preview = false
    state.press_pointer = nil
    state.start_at = nil
    state.candidates = nil
    if state.binding then state.binding:remove() end
    if state.close_binding then state.close_binding:remove() end
    state.binding = hl.on("enzui.pointer_button", function(button, value)
        if button ~= 272 then return end
        if value == 1 then press() else release() end
    end)
    state.close_binding = hl.on("window.close", function(window)
        local address = window and window.address
        if not address then return end
        if state.window and state.window.address == address then
            state.generation = (state.generation or 0) + 1
            stop_timer()
            hide_preview()
            state.window = nil
            state.press_pointer = nil
            state.start_at = nil
            state.candidates = nil
        end
    end)
    assert(state.binding and state.close_binding, "Drag-maximize pointer bindings were rejected")
    return true
end

function enzui_drag_maximize_theme(accent, radius)
    assert(type(accent) == "string" and accent:match("^%x%x%x%x%x%x$"), "Invalid preview accent")
    assert(type(radius) == "number" and radius >= 0 and radius <= 512, "Invalid preview radius")
    enzui_drag_maximize_accent = accent
    enzui_drag_maximize_radius = math.floor(radius)
    -- Window borders follow the accent where the border preferences use it.
    if enzui_apply_window_borders then
        enzui_apply_window_borders()
    elseif hl.window_rule then
        -- Before the first preferences are applied: the focused tile takes the accent.
        pcall(hl.window_rule, { name = "enzui-tiled-focus", match = { float = false },
            border_color = "rgba(" .. accent .. "cc) rgba(55585d33)" })
    end
    return true
end
