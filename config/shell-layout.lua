-- Move shell panels to the output workspace before native bar registration; retry unfinished
-- moves.
function enzui_panels_onto(output,owner)
    local monitor
    for _,candidate in ipairs(hl.get_monitors()) do
        if candidate.name==output and candidate.enabled then monitor=candidate;break end
    end
    assert(monitor and not monitor.is_mirror, "Unsupported shell output")
    local elsewhere=false
    for _,window in ipairs(hl.get_windows()) do
        if window.mapped and window.class=="dev.enzui.Desktop" and window.pid==owner and not (window.monitor and window.monitor.name==output) then
            local result=hl.dispatch(hl.dsp.window.move({window=window,monitor=output,follow=false}))
            assert(result and result.ok==true, "Could not move desktop panel")
            result=hl.dispatch(hl.dsp.window.move({window=window,x=monitor.x,y=monitor.y}))
            assert(result and result.ok==true, "Could not position desktop panel")
            elsewhere=elsewhere or not (window.monitor and window.monitor.name==output)
        end
    end
    return elsewhere
end

-- Clamp stranded floating windows to the work area; shell and sidebar windows keep their own
-- policies.
local function enzui_fit_windows(monitor,reserved,panels,only)
    local x0,y0,w0,h0=enzui_app_work_area(monitor,reserved)
    if w0<80 or h0<80 then return end
    local skip={}
    for _,panel in ipairs(panels) do skip[panel.pid]=true end
    local held={}
    for _,e in ipairs(enzui_sidebar and enzui_sidebar.entries or {}) do
        local ok,address=pcall(function() return e.window.address end)
        if ok and address then held[address]=true end
    end
    for _,w in ipairs(hl.get_windows()) do
        if (not only or (w.address == only.address and w.pid == only.pid)) and w.mapped and w.floating and not w.pinned and not skip[w.pid] and not held[w.address]
            and not (type(w.fullscreen)=="number" and w.fullscreen>0)
            and w.monitor and w.monitor.name==monitor.name and w.workspace and type(w.workspace.id)=="number" and w.workspace.id>0 then
            local width,height=math.min(w.size.x,w0),math.min(w.size.y,h0)
            local x=math.max(x0,math.min(w.at.x,x0+w0-width))
            local y=math.max(y0,math.min(w.at.y,y0+h0-height))
            if width~=w.size.x or height~=w.size.y then hl.dispatch(hl.dsp.window.resize({window=w,x=width,y=height})) end
            if x~=w.at.x or y~=w.at.y then hl.dispatch(hl.dsp.window.move({window=w,x=x,y=y})) end
        end
    end
end

function enzui_apply_shell_layout(output,edge,metrics)
    metrics=enzui_resolve_metrics(metrics)
    assert(type(output)=="string" and output~="" and not output:match("^ENZREMOTE%-"), "Explicit desktop output required")
    assert(edge=="top" or edge=="bottom", "Invalid bar position")
    local monitor
    for _,candidate in ipairs(hl.get_monitors()) do
        if candidate.name==output and candidate.enabled then monitor=candidate;break end
    end
    assert(monitor and not monitor.is_mirror, "Unsupported shell output")
    local width,height=enzui_output_w(monitor),enzui_output_h(monitor)
    assert(width>=320 and height>=240 and width<=16384 and height<=16384, "Unsupported shell geometry")
    local panels={}
    for _,window in ipairs(hl.get_windows()) do
        if window.mapped and window.class=="dev.enzui.Desktop" then
            panels[#panels+1]=window
        end
    end
    local offset=enzui_bar_offset(metrics)
    local y=edge=="top" and offset or height-metrics.bar-offset
    local sidebar=enzui_sidebar
    local configured=sidebar and sidebar.output==output
    local behavior,occupied=nil,false
    if configured and enzui_sidebar_reservation_inputs then
        behavior,occupied=enzui_sidebar_reservation_inputs()
    elseif configured then
        behavior,occupied=sidebar.enabled~=false and sidebar.behavior or nil,sidebar.enabled~=false and (#sidebar.entries>0 or sidebar.error~="")
    end
    local reserved=enzui_sidebar_reservations(metrics,edge,configured and sidebar.side or "left",behavior,occupied)
    enzui_apply_reservations(monitor,reserved)
    -- An output that no longer hosts the shell gives its band back.
    for _,other in ipairs(hl.get_monitors()) do
        local band=other.reserved
        if other.name~=output and other.enabled and band and not other.name:match("^ENZREMOTE%-")
            and (band.left~=0 or band.right~=0 or band.top~=0 or band.bottom~=0) then
            pcall(enzui_apply_reservations,other,{left=0,right=0,top=0,bottom=0})
        end
    end
    pcall(enzui_fit_windows,monitor,reserved,panels)
    hl.window_rule({name="desktop-panel",match={class="^dev[.]enzui[.]Desktop$"},monitor=output,
        move=string.format("0 %.0f",y),size=string.format("%.0f %d",width,metrics.bar),
        min_size=string.format("%.0f %d",width,metrics.bar),max_size=string.format("%.0f %d",width,metrics.bar)})
    for _,window in ipairs(panels) do
        local scene=hl.plugin and hl.plugin.enzui_sidebar_scene
        if scene and scene.version()==8 then
            assert(scene.anchor_panel(window.address,window.pid,monitor.x,monitor.y+y,width,metrics.bar,edge=="bottom"),"Bar anchor rejected")
        end
        -- Sized first, then placed: a resize keeps a floating window's centre, so a bar
        -- that grew or shrank sat half the difference off its edge until the anchor
        -- put it back a frame later.
        local resized=window.size.x~=width or window.size.y~=metrics.bar
        if resized then
            local result=hl.dispatch(hl.dsp.window.resize({window=window,x=width,y=metrics.bar}))
            assert(result and result.ok==true, "Could not size desktop panel")
        end
        if resized or window.at.x~=monitor.x or window.at.y~=monitor.y+y then
            local result=hl.dispatch(hl.dsp.window.move({window=window,x=monitor.x,y=monitor.y+y}))
            assert(result and result.ok==true, "Could not position desktop panel")
        end
    end
    enzui_bar_edge=edge
    return {ok=true}
end

-- Bound clients that ignore initial xdg configure bounds before their next presentation.
if hl.on and not enzui_float_admission then
    enzui_float_admission=hl.on("window.open",function(window)
            if not window.mapped then return end
            local monitor=window.monitor
            if not monitor or monitor.name:match("^ENZREMOTE%-") or monitor.is_mirror then return end
            local panels={}
            for _,w in ipairs(hl.get_windows()) do
                if w.class=="dev.enzui.Desktop" then panels[#panels+1]=w end
            end
            enzui_fit_windows(monitor,monitor.reserved or {left=0,right=0,top=0,bottom=0},panels,window)
    end)
end
