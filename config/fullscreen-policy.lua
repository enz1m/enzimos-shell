local chrome_tag = "enzui-fullscreen-chrome-hidden"
local idle_tag = "enzui-fullscreen-idle"
local shell_helpers = {
    ["dev.enzui.AgentConsent"] = true,
    ["dev.enzui.Authentication"] = true,
    ["dev.enzui.FileChooser"] = true,
    ["dev.enzui.KeyringPrompt"] = true,
    ["dev.enzui.NetworkSecrets"] = true,
    ["dev.enzui.PortalAccess"] = true,
    ["dev.enzui.RemotePlaceholder"] = true,
    ["dev.enzui.SharePicker"] = true,
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
    if has_tag(window, tag) ~= enabled then
        hl.dispatch(hl.dsp.window.tag({ window = window, tag = (enabled and "+" or "-") .. tag }))
    end
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

local function is_fullscreen_app(window, shell_owners)
    if not window.mapped or window.hidden or window.fullscreen ~= 2
        or shell_helpers[window.class] or shell_owners[window.pid] then
        return false
    end
    for _, tag in ipairs(window.tags or {}) do
        if tag == "enzui-sidebar-app" or tag:match("^enzremote%-") then return false end
    end
    return true
end

local function refresh(closing)
    local windows, shell_owners, covered = hl.get_windows(), {}, {}
    for _, window in ipairs(windows) do
        if window.class == "dev.enzui.Desktop" and window.pid and window.pid > 0 then
            shell_owners[window.pid] = true
        end
    end
    for _, window in ipairs(windows) do
        local workspace = window.workspace
        local monitor = workspace and workspace.monitor
        local fullscreen = window ~= closing and monitor ~= nil and not monitor.name:match("^ENZREMOTE%-")
            and is_fullscreen_app(window, shell_owners)
        set_tag(window, idle_tag, fullscreen)
        if fullscreen and hl.get_active_workspace(monitor.name) == workspace then
            covered[monitor.name] = true
        end
    end
    for _, window in ipairs(windows) do
        if window.class == "dev.enzui.Desktop" or window.class == "dev.enzui.AssistantTrigger" then
            set_tag(window, chrome_tag, window.monitor ~= nil and covered[window.monitor.name] == true)
        end
    end
end

hl.on("window.close", function(window)
    if enzui_search_presented == window then enzui_search_presented = nil end
    refresh(window)
end)
local move_refresh
hl.on("window.move_to_workspace", function()
    -- The move event precedes the window's monitor update.
    if not move_refresh then
        move_refresh = hl.timer(function()
            move_refresh = nil
            refresh()
        end, { timeout = 1, type = "oneshot" })
    end
end)
for _, event in ipairs({
    "window.open", "window.class", "window.fullscreen", "window.minimize",
    "workspace.active", "workspace.move_to_monitor", "monitor.layout_changed",
}) do
    hl.on(event, function() refresh() end)
end
refresh()
