local chrome_tag = "enzui-fullscreen-chrome-hidden"
local borderless_tag = "enzui-borderless-fullscreen"
local idle_tag = "enzui-fullscreen-idle"
local shell_helpers = {
    ["dev.enzui.AgentConsent"] = true,
    ["dev.enzui.AssistantTrigger"] = true,
    ["dev.enzui.Authentication"] = true,
    ["dev.enzui.Desktop"] = true,
    ["dev.enzui.Dock"] = true,
    ["dev.enzui.FileChooser"] = true,
    ["dev.enzui.KeyringPrompt"] = true,
    ["dev.enzui.Launcher"] = true,
    ["dev.enzui.NetworkSecrets"] = true,
    ["dev.enzui.NotificationCenter"] = true,
    ["dev.enzui.NotificationToasts"] = true,
    ["dev.enzui.PluginPanel"] = true,
    ["dev.enzui.PortalPrint"] = true,
    ["dev.enzui.PortalAccess"] = true,
    ["dev.enzui.QuickSearch"] = true,
    ["dev.enzui.RemotePlaceholder"] = true,
    ["dev.enzui.SharePicker"] = true,
    ["dev.enzui.WorkspaceOverview"] = true,
}

hl.window_rule({
    name = "desktop-fullscreen-chrome",
    match = { tag = chrome_tag },
    opacity = "0 override 0 override",
    no_focus = true,
})
hl.window_rule({
    name = "desktop-fullscreen-idle",
    match = { tag = idle_tag, fullscreen_state_internal = 2 },
    idle_inhibit = "fullscreen",
})

local function has_tag(window, tag)
    for _, value in ipairs(window.tags or {}) do
        if value == tag then return true end
    end
    return false
end

local function set_tag(window, tag, enabled)
    if has_tag(window, tag) == enabled then return false end
    hl.dispatch(hl.dsp.window.tag({ window = window, tag = (enabled and "+" or "-") .. tag }))
    return true
end

function enzui_fullscreen_chrome_hidden(window)
    return has_tag(window, chrome_tag)
end

function enzui_desktop_fullscreen()
    for _, window in ipairs(hl.get_windows()) do
        if window.class == "dev.enzui.Desktop" and enzui_fullscreen_chrome_hidden(window) then return true end
    end
    return false
end

local function is_desktop_app(window, shell_owners)
    if not window or not window.mapped or window.hidden
        or shell_helpers[window.class] or shell_owners[window.pid] then
        return false
    end
    for _, tag in ipairs(window.tags or {}) do
        if tag == "enzui-sidebar-app" or tag:match("^enzremote%-") then return false end
    end
    return true
end

local function shell_pids(windows)
    local owners = {}
    for _, window in ipairs(windows) do
        if window.class == "dev.enzui.Desktop" and window.pid and window.pid > 0 then
            owners[window.pid] = true
        end
    end
    return owners
end

local function local_output(window)
    local monitor = window.workspace and window.workspace.monitor
    if monitor and not monitor.is_mirror and not monitor.name:match("^ENZREMOTE%-") then return monitor end
end

function enzui_fullscreen_focused()
    local window = hl.get_active_window()
    return window ~= nil and (window.fullscreen == 2 or window.borderless_fullscreen == true)
        and local_output(window) ~= nil and is_desktop_app(window, shell_pids(hl.get_windows()))
end

function enzui_fullscreen_covers(window)
    return window.fullscreen == 2 or has_tag(window, borderless_tag)
end

local function refresh(closing)
    local windows, active, covered = hl.get_windows(), hl.get_active_window(), {}
    local shell_owners = shell_pids(windows)
    local borderless_changed = false
    for _, window in ipairs(windows) do
        local monitor = local_output(window)
        local eligible = window ~= closing and monitor ~= nil and is_desktop_app(window, shell_owners)
        local fullscreen = eligible and window.fullscreen == 2
        local borderless = eligible and window == active and window.borderless_fullscreen == true
        local on_desktop = (fullscreen or borderless) and hl.get_active_workspace(monitor.name) == window.workspace
        borderless = borderless and on_desktop
        borderless_changed = set_tag(window, borderless_tag, borderless) or borderless_changed
        set_tag(window, idle_tag, fullscreen)
        if borderless or fullscreen and on_desktop then
            covered[monitor.name] = true
        end
    end
    for _, window in ipairs(windows) do
        if window.class == "dev.enzui.Desktop" or window.class == "dev.enzui.AssistantTrigger" then
            set_tag(window, chrome_tag, window.monitor ~= nil and covered[window.monitor.name] == true)
        end
    end
    if borderless_changed then
        if enzui_sidebar_wake then enzui_sidebar_wake() end
        hl.dispatch(hl.dsp.event("enzui-dock-changed"))
    end
end

hl.on("window.close", function(window)
    if enzui_search_presented == window then enzui_search_presented = nil end
    refresh(window)
end)
local pending_refresh
local function schedule_refresh()
    if pending_refresh then return end
    pending_refresh = hl.timer(function()
        pending_refresh = nil
        refresh()
    end, { timeout = 80, type = "oneshot" })
end

for _, event in ipairs({
    "window.active", "window.borderless_fullscreen", "window.update_rules", "window.move_to_workspace",
}) do
    hl.on(event, schedule_refresh)
end
for _, event in ipairs({
    "window.open", "window.class", "window.fullscreen", "window.minimize",
    "workspace.active", "workspace.move_to_monitor", "monitor.layout_changed",
}) do
    hl.on(event, function() refresh() end)
end
refresh()
