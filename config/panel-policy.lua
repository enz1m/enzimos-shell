hl.window_rule({
    name = "desktop-panel", match = { class = "^dev[.]enzui[.]Desktop$" },
    no_xdg_drags = true, suppress_event = "maximize fullscreen",
})

if enzui_panel_policy_installed ~= 17 then
    local function panel(window)
        return window and window.mapped and not window.hidden
            and (window.class == "dev.enzui.Dock" or window.class == "dev.enzui.Desktop" or window.class == "dev.enzui.Shortcuts" or window.class == "dev.enzui.QuickSearch" or window.class == "dev.enzui.WorkspaceOverview" or window.class == "dev.enzui.ControlCenter" or window.class == "dev.enzui.PluginPanel"
                or window.class == "dev.enzui.NotificationCenter" or window.class == "dev.enzui.NotificationToasts"
                or window.class == "dev.enzui.AssistantPanel" or window.class == "dev.enzui.AssistantTrigger"
                or (window.class == "dev.enzui.Sidebar" and enzui_sidebar and (enzui_sidebar.reveal or #enzui_sidebar.entries>0 or enzui_sidebar.error~="")))
    end
    local function panel_at_pointer()
        local point = hl.get_cursor_pos()
        for _, window in ipairs(hl.get_windows()) do
            if window.class=="dev.enzui.Dock" then
                if enzui_dock_pointer_inside and enzui_dock_pointer_inside(window,point) then return true end
            elseif panel(window) and not (enzui_fullscreen_chrome_hidden and enzui_fullscreen_chrome_hidden(window)) then
                local api=hl.plugin and hl.plugin.enzui_sidebar_scene
                local bar_state=window.class=="dev.enzui.Desktop" and api and api.bar_regions_state and api.bar_regions_state(window.pid)
                local at, size = window.at, window.size
                if not (bar_state and bar_state.suppressed) and point.x >= at.x - 2 and point.y >= at.y - 2
                    and point.x < at.x + size.x + 2 and point.y < at.y + size.y + 2 then
                    return true
                end
            end
        end
        return false
    end
    local function focused(action)
        return function()
            local window = hl.get_active_window()
            if window and not panel(window) then return hl.dispatch(action) end
        end
    end
    local function mouse(action)
        local awaiting_release = false
        return function()
            if awaiting_release then
                awaiting_release = false
                local result=hl.dispatch(action)
                if enzui_sidebar_release and not enzui_sidebar_pointer_binding then enzui_sidebar_release() end
                return result
            end
            if panel_at_pointer() then return {ok=true, pass_event=true} end
            if enzui_sidebar_press and not enzui_sidebar_pointer_binding then enzui_sidebar_press() end
            local result = hl.dispatch(action)
            awaiting_release = result and result.request_release == true
            return result
        end
    end
    local function bind(key, action, options)
        hl.unbind(key)
        hl.bind(key, action, options)
    end
    -- A close request, so apps can ask about unsaved work; never a kill.
    bind("SUPER + Q", focused(hl.dsp.window.close()))
    bind("ALT + F4", focused(hl.dsp.window.close()))
    bind("SUPER + V", focused(hl.dsp.window.float({action="toggle"})))
    bind("SUPER + F", focused(hl.dsp.window.fullscreen()))
    bind("SUPER + M", focused(hl.dsp.window.fullscreen({mode="maximized"})))
    for i=1,9 do
        bind("SUPER + SHIFT + code:" .. (i + 9), focused(function()
            local window=hl.get_active_window()
            local id=i
            if enzui_workspace_id_at then
                if not window or not window.monitor then return end
                id=enzui_workspace_id_at(window.monitor.name,i)
            end
            if id then return hl.dispatch(hl.dsp.window.move({workspace=id})) end
        end))
    end
    for _, spec in ipairs({{"Prior",-1},{"Next",1}}) do
        bind("SUPER + SHIFT + " .. spec[1], focused(function()
            local window = hl.get_active_window()
            if not window.workspace or not window.monitor or not enzui_workspace_neighbor then return end
            local id = enzui_workspace_neighbor(window.monitor.name, window.workspace.id, spec[2])
            if id then return hl.dispatch(hl.dsp.window.move({workspace=id})) end
        end))
    end
    bind("SUPER + mouse:272", mouse(hl.dsp.window.drag()), {mouse=true})
    bind("SUPER + mouse:273", mouse(hl.dsp.window.resize()), {mouse=true})
    enzui_panel_policy_installed = 17
end
