if enzui_control_dismiss_version ~= 8 then
    for _,binding in ipairs(enzui_control_center_dismiss or {}) do binding:remove() end
    local function inside(window,point)
        local at,size=window.at,window.size
        if point.x<at.x or point.y<at.y or point.x>=at.x+size.x or point.y>=at.y+size.y then return false end
        local api=hl.plugin and hl.plugin.enzui_sidebar_scene
        if api and type(api.input_at)=="function" then
            local hit=api.input_at(window.address,window.pid,point.x,point.y)
            if type(hit)=="boolean" then return hit end
        end
        return true
    end
    local function dismiss_outside()
        local point = hl.get_cursor_pos()
        local windows = hl.get_windows()
        local spread = {}
        for _, window in ipairs(windows) do
            if window.mapped and window.class == "dev.enzui.Desktop" and inside(window,point) then return end
            if window.mapped and window.class == "dev.enzui.WindowSpread" then spread[window.pid] = true end
        end
        for _, window in ipairs(windows) do
            local api=hl.plugin and hl.plugin.enzui_sidebar_scene
            local shaped_popup=(window.class=="dev.enzui.QuickSearch" or window.class=="dev.enzui.Launcher") and window.workspace and type(window.workspace.id)=="number"
                and api and type(api.input_at)=="function" and type(api.input_at(window.address,window.pid,point.x,point.y))=="boolean"
            -- The overview's spread handles presses around its Search itself.
            if shaped_popup and window.class == "dev.enzui.QuickSearch" and spread[window.pid] then shaped_popup = false end
            if window.mapped and (shaped_popup or window.class == "dev.enzui.ControlCenter" or window.class == "dev.enzui.WorkspaceOverview" or window.class == "dev.enzui.PluginPanel" or window.class == "dev.enzui.NotificationCenter") then
                if not inside(window,point) then hl.dispatch(hl.dsp.window.close({window=window})) end
            end
        end
    end
    enzui_control_center_dismiss = {
        hl.bind("mouse:272", dismiss_outside, { non_consuming = true, ignore_mods = true }),
        hl.bind("mouse:273", dismiss_outside, { non_consuming = true, ignore_mods = true }),
    }
    enzui_control_dismiss_version = 8
end

function enzui_control_center_cache(owner,visible)
    local target
    for _,w in ipairs(hl.get_windows()) do
        if w.mapped and w.pid==owner and w.class=="dev.enzui.ControlCenter" then
            if target then return false end
            target=w
        end
    end
    if not target or not target.monitor then return false end
    local current=hl.get_active_window()
    local was_focused=current and current.address==target.address
    local ws=hl.get_active_workspace(target.monitor.name)
    if not ws or type(ws.id)~="number" then return false end
    local function focus(w)
        local no_warps=hl.get_config("cursor.no_warps")
        hl.config({cursor={no_warps=true}})
        local ok,err=pcall(function()hl.dispatch(hl.dsp.focus({window=w}))end)
        hl.config({cursor={no_warps=no_warps}})
        if not ok then error(err) end
    end
    hl.dispatch(hl.dsp.window.set_prop({prop="render_unfocused",value="0",window=target}))
    local preparing=visible=="prepare"
    local showing=visible==true
    local api=hl.plugin and hl.plugin.enzui_sidebar_scene
    local frames=api and type(api.panel_frame)=="function" and api.owner_alive(owner)
    local shown=enzui_control_shown
    local already_shown=shown and shown.owner==owner and shown.stable_id==target.stable_id
    if showing then enzui_control_shown={owner=owner,stable_id=target.stable_id}
    elseif not preparing and shown and shown.owner==owner then enzui_control_shown=nil end
    -- Older providers retain immediate reveal; current providers reveal only a fresh buffer.
    local opacity=(showing or (preparing and (already_shown or not frames))) and "1" or "0"
    hl.dispatch(hl.dsp.window.set_prop({prop="opacity",value=opacity,window=target}))
    hl.dispatch(hl.dsp.window.set_prop({prop="opacity_inactive",value=opacity,window=target}))
    if showing then
        if not was_focused then
            local previous
            for _,w in ipairs(hl.get_windows()) do
                if w.mapped and not w.hidden and w.pid~=owner and w.monitor and w.monitor.name==target.monitor.name
                    and w.workspace and w.workspace.id==ws.id
                    and (not previous or w.focus_history_id<previous.focus_history_id) then previous=w end
            end
            enzui_control_return=previous and {owner=owner,window=previous,stable_id=previous.stable_id} or nil
        end
        if not target.workspace or target.workspace.id~=ws.id then
            hl.dispatch(hl.dsp.window.move({window=target,workspace=ws.id,follow=false}))
        end
        hl.dispatch(hl.dsp.window.pin({window=target,action="on"}))
        focus(target)
        hl.dispatch(hl.dsp.window.alter_zorder({window=target,mode="top"}))
    elseif not (preparing and already_shown) then
        -- Retain the live workspace and empty input region. Opacity also hides
        -- the last submitted buffer if the renderer reuses an empty frame.
        hl.dispatch(hl.dsp.window.pin({window=target,action="on"}))
        local previous=enzui_control_return
        if was_focused and previous and previous.owner==owner then
            pcall(function()
                local w=previous.window
                if w.mapped and not w.hidden and w.stable_id==previous.stable_id and w.monitor
                    and w.monitor.name==target.monitor.name and w.workspace and w.workspace.id==ws.id then focus(w) end
            end)
        end
        enzui_control_return=nil
    end
    if frames then assert(api.panel_frame(target.address,owner,preparing),"Panel frame preparation rejected") end
    return true
end
