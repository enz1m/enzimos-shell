local function search_surface(owner)
    assert(type(owner)=="number" and owner%1==0 and owner>1,"Invalid search owner")
    local found
    for _,w in ipairs(hl.get_windows()) do
        if w.mapped and w.pid==owner and w.class=="dev.enzui.QuickSearch" then
            assert(not found,"Ambiguous search surface")
            found=w
        end
    end
    assert(found,"Search surface is unavailable")
    return found
end
local function dispatch(action)
    local result=hl.dispatch(action)
    assert(result and result.ok,"Search window operation rejected")
end
local function pointer_position()
    local p=hl.get_cursor_pos()
    assert(p and type(p.x)=="number" and type(p.y)=="number" and p.x==p.x and p.y==p.y and math.abs(p.x)<=1000000 and math.abs(p.y)<=1000000,"Invalid cursor position")
    return {x=p.x,y=p.y}
end
local function preserve_pointer(action)
    local pointer=pointer_position()
    local previous=hl.get_config("cursor.no_warps")
    assert(type(previous)=="boolean","Missing cursor warp policy")
    if not previous then hl.config({cursor={no_warps=true}}) end
    local ok,result=pcall(action)
    if not previous then hl.config({cursor={no_warps=false}}) end
    assert(ok,result)
    local current=pointer_position()
    assert(current.x==pointer.x and current.y==pointer.y,"Search operation moved the pointer")
    return result
end
local function refresh_pointer_focus()
    local pointer=pointer_position()
    local previous=hl.get_config("input.follow_mouse")
    assert(type(previous)=="number" and previous%1==0 and previous>=0 and previous<=3,"Missing pointer focus policy")
    if previous~=3 then hl.config({input={follow_mouse=3}}) end
    local ok,result=pcall(function() dispatch(hl.dsp.cursor.move({x=pointer.x,y=pointer.y})) end)
    if previous~=3 then hl.config({input={follow_mouse=previous}}) end
    assert(ok,result)
end
local function local_window(w)
    if not w or not w.mapped or not w.monitor or w.monitor.name:match("^ENZREMOTE%-") then return false end
    for _,tag in ipairs(w.tags or {}) do if tag:match("^enzremote%-") then return false end end
    return true
end
local function transition(owner,epoch)
    assert(type(owner)=="number" and owner%1==0 and owner>1,"Invalid search owner")
    assert(type(epoch)=="number" and epoch%1==0 and epoch>0 and epoch<=9007199254740991,"Invalid search transition")
    if not enzui_search_epoch or enzui_search_epoch.owner~=owner then
        enzui_search_epoch={owner=owner,value=0}
        enzui_search_return=nil
    end
    assert(epoch>=enzui_search_epoch.value,"Stale search transition")
    enzui_search_epoch.value=epoch
end
local function placement(m,width,height,island,owner)
    assert(type(m.scale)=="number" and m.scale>0 and m.scale<=8,"Invalid search output scale")
    if island~=nil then
        assert(type(island)=="table" and type(island.bottom)=="boolean","Invalid Island Search placement")
        for _,key in ipairs({"x","bar_end","side_gap","far_gap"}) do
            local n=island[key]
            assert(type(n)=="number" and n==n and math.abs(n)<=32768,"Invalid Island Search coordinate")
        end
        local sw,sh=enzui_output_w(m),enzui_output_h(m)
        -- The window keeps shadow room around the card; the card is what must fit.
        local ml,mt,mr,mb=island.ml or 0,island.mt or 0,island.mr or 0,island.mb or 0
        for _,n in ipairs({ml,mt,mr,mb}) do
            assert(type(n)=="number" and n>=0 and n<=64,"Invalid Island Search shadow margin")
        end
        local x=island.x
        local width,height=width-ml-mr,height-mt-mb
        local y=island.bottom and sh-island.bar_end-height or island.bar_end
        assert(island.side_gap>=0 and island.far_gap>=0 and island.bar_end>=0
            and x>=island.side_gap and x+width<=sw-island.side_gap
            and y>=0 and y+height<=sh,"Island Search exceeds output")
        assert(island.bottom and y>=island.far_gap or not island.bottom and y+height<=sh-island.far_gap,"Island Search exceeds work area")
        local scene=hl.plugin and hl.plugin.enzui_sidebar_scene
        -- Islands take their input through the bar regions; the frame's bar
        -- is a plain window.
        if not island.frame then
            local state=scene and type(scene.bar_regions_state)=="function" and scene.bar_regions_state(owner)
            assert(state and state.enabled and not state.pending,"Island Search input service is not ready")
        end
        -- The scene keeps the card's edge at the bar while it resizes, at
        -- either edge: placed by plain moves, a tall card under a top bar was
        -- held by its bottom edge for a frame and jumped over the bar.
        assert(scene and type(scene.anchor_panel)=="function","Island Search anchor service is unavailable")
        return m.x+x-ml,m.y+y-mt,scene
    end
    local r=m.reserved
    assert(r and m.scale>0,"Missing search work area")
    local aw,ah=enzui_output_w(m)-r.left-r.right,enzui_output_h(m)-r.top-r.bottom
    local top=math.floor(math.min(96,math.max(16,ah*0.12)))
    assert(width<=aw and height<=ah-top-16,"Search surface exceeds work area")
    return math.floor(m.x+r.left+(aw-width)/2),math.floor(m.y+r.top+top),nil
end
local function search_dim(owner,w,epoch,dim,fade)
    local scene=hl.plugin and hl.plugin.enzui_sidebar_scene
    if scene and type(scene.search_dim)=="function" then
        dispatch(hl.dsp.window.set_prop({window=w,prop="dim_around",value="0"}))
        assert(scene.search_dim(owner,w.address,epoch,dim,fade),"Search background fade rejected")
    else
        dispatch(hl.dsp.window.set_prop({window=w,prop="dim_around",value=dim and "1" or "0"}))
    end
end
function enzui_search_hide_dim(owner,epoch,fade)
    transition(owner,epoch)
    local w=search_surface(owner)
    assert(local_window(w),"Search surface changed ownership")
    enzui_search_background_fade=fade~=false
    search_dim(owner,w,epoch,false,enzui_search_background_fade)
    return {ok=true}
end
function enzui_search_present(owner,output,width,height,epoch,island,fade)
    transition(owner,epoch)
    local w=search_surface(owner)
    assert(local_window(w),"Search surface changed ownership")
    local m
    for _,candidate in ipairs(hl.get_monitors()) do if candidate.name==output and candidate.enabled then m=candidate end end
    assert(m and not m.is_mirror and not output:match("^ENZREMOTE%-"),"Search output unavailable")
    assert(type(width)=="number" and type(height)=="number" and width%1==0 and height%1==0 and width>=240 and width<=1024 and height>=80 and height<=900,"Invalid search dimensions")
    local x,y,scene=placement(m,width,height,island,owner)
    local ws=hl.get_active_workspace(output)
    assert(ws and type(ws.id)=="number","No desktop for search")
    local previous=hl.get_active_window()
    if local_window(previous) and previous~=w and previous.pid~=owner and previous.workspace and type(previous.workspace.id)=="number" then
        enzui_search_return={window=previous,stable_id=previous.stable_id,pid=previous.pid,owner=owner}
    end
    preserve_pointer(function()
        dispatch(hl.dsp.window.move({window=w,workspace=ws.id,follow=false}))
        if scene then assert(scene.anchor_panel(w.address,owner,x,y,width,height,island.bottom),"Island Search anchor rejected") end
        dispatch(hl.dsp.window.resize({window=w,x=width,y=height}))
        dispatch(hl.dsp.window.move({window=w,x=x,y=y}))
        dispatch(hl.dsp.window.alter_zorder({window=w,mode="top"}))
        local dim=island==nil or island.dim==true
        enzui_search_background_fade=fade~=false
        search_dim(owner,w,epoch,dim,enzui_search_background_fade)
        dispatch(hl.dsp.focus({window=w}))
    end)
    enzui_search_presented = w
    return {ok=true}
end
function enzui_search_resize(owner,output,width,height,epoch,island)
    assert(enzui_search_epoch and enzui_search_epoch.owner==owner and enzui_search_epoch.value==epoch,"Stale search resize")
    local w=search_surface(owner)
    assert(local_window(w) and w.workspace and type(w.workspace.id)=="number","Search is not presented")
    local m
    for _,candidate in ipairs(hl.get_monitors()) do if candidate.name==output and candidate.enabled then m=candidate end end
    assert(m and not m.is_mirror and not output:match("^ENZREMOTE%-"),"Search output unavailable")
    assert(type(width)=="number" and type(height)=="number" and width%1==0 and height%1==0 and width>=240 and width<=1024 and height>=80 and height<=900,"Invalid search dimensions")
    local x,y,scene=placement(m,width,height,island,owner)
    preserve_pointer(function()
        if not scene then
            dispatch(hl.dsp.window.resize({window=w,x=width,y=height}))
            dispatch(hl.dsp.window.move({window=w,x=x,y=y}))
            return
        end
        -- The compositor resizes a floating window about its centre, which moved the card by
        -- half the change until the scene's anchor caught up a frame later. In the same call,
        -- put it where the anchor holds it until the new buffer lands: at the bar's edge, with
        -- the shown picture (the old height while growing) touching it.
        local at_x,shown=w.at.x,math.min(w.size.y,height)
        assert(scene.anchor_panel(w.address,owner,x,y,width,height,island.bottom),"Island Search anchor rejected")
        dispatch(hl.dsp.window.resize({window=w,x=width,y=height}))
        dispatch(hl.dsp.window.move({window=w,x=at_x,y=island.bottom and y+height-shown or y}))
    end)
    return {ok=true}
end
-- The overview keeps Search open across a desktop switch: the card moves to the
-- new desktop and keeps the keyboard. Its return target was on the old desktop.
function enzui_search_follow(owner,epoch)
    assert(enzui_search_epoch and enzui_search_epoch.owner==owner and enzui_search_epoch.value==epoch,"Stale search follow")
    local w=search_surface(owner)
    assert(local_window(w) and w.workspace and type(w.workspace.id)=="number","Search is not presented")
    local ws=hl.get_active_workspace(w.monitor.name)
    assert(ws and type(ws.id)=="number","No desktop for search")
    enzui_search_return=nil
    if w.workspace.id~=ws.id then
        preserve_pointer(function()
            dispatch(hl.dsp.window.move({window=w,workspace=ws.id,follow=false}))
            dispatch(hl.dsp.window.alter_zorder({window=w,mode="top"}))
            dispatch(hl.dsp.focus({window=w}))
        end)
    end
    return {ok=true}
end
function enzui_search_park(owner,epoch)
    transition(owner,epoch)
    local w=search_surface(owner)
    assert(local_window(w),"Search surface changed ownership")
    enzui_search_presented = nil
    local target
    if hl.get_active_window()==w then
        local saved=enzui_search_return
        if saved and saved.owner==owner then
            local ok,valid=pcall(function() return local_window(saved.window) and saved.window.stable_id==saved.stable_id and saved.window.pid==saved.pid and saved.window.workspace and type(saved.window.workspace.id)=="number" end)
            if ok and valid then target=saved.window end
        end
        for _,candidate in ipairs(hl.get_windows()) do
            if local_window(candidate) and candidate.workspace and type(candidate.workspace.id)=="number" then
                if not target and candidate.pid==owner and candidate.class=="dev.enzui.Desktop" then target=candidate end
            end
        end
    end
    preserve_pointer(function()
        search_dim(owner,w,epoch,false,enzui_search_background_fade~=false)
        dispatch(hl.dsp.window.move({window=w,workspace="special:enzui-search",follow=false}))
        if target then dispatch(hl.dsp.focus({window=target})) end
        refresh_pointer_focus()
    end)
    return {ok=true}
end
function enzui_search_focus_window(address,pid,stable_id,sidebar)
    local w=hl.get_window("address:"..address)
    assert(local_window(w) and w.pid==pid and w.stable_id==stable_id,"Search result window changed")
    if sidebar then
        local entry
        for _,candidate in ipairs(enzui_sidebar and enzui_sidebar.entries or {}) do
            if candidate.id==sidebar and candidate.pid==pid and candidate.window.address==address then entry=candidate;break end
        end
        assert(entry and not entry.restore_pending and enzui_sidebar.restoring~=sidebar,"Pinned app changed or is restoring")
        if enzui_sidebar.shown~=sidebar or (enzui_sidebar.after_close and enzui_sidebar.after_close.park) then
            local result=enzui_sidebar_toggle(sidebar)
            assert(result and result.ok,"Pinned app could not resume")
            return result
        end
        assert(not enzui_sidebar.after_close,"Pinned app is returning to the desktop")
        if enzui_sidebar.native and hl.plugin.enzui_sidebar_scene.status(address)~="open" then return {ok=true} end
    end
    assert(w.workspace and type(w.workspace.id)=="number","Window is not on a desktop")
    preserve_pointer(function()
        -- Floating apps keep their above-fullscreen grant after losing focus.
        -- Returning to the maximized app must lower those grants as well as
        -- changing focus; pinned shell surfaces and its own dialogs stay above.
        if w.fullscreen and w.fullscreen>0 then
            local windows=hl.get_windows()
            for i=#windows,1,-1 do
                local other=windows[i]
                if local_window(other) and other.workspace and other.workspace.id==w.workspace.id
                    and other.floating and not other.pinned and other.pid~=w.pid
                    and (not enzui_search_epoch or other.pid~=enzui_search_epoch.owner) then
                    dispatch(hl.dsp.window.alter_zorder({window=other,mode="bottom"}))
                end
            end
        end
        dispatch(hl.dsp.window.alter_zorder({window=w,mode="top"}))
        dispatch(hl.dsp.focus({window=w}))
    end)
    return {ok=true}
end
