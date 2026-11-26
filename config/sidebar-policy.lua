enzui_sidebar = enzui_sidebar or {entries={},next_id=1,side="left",bar="top",shown=nil,reveal=false,heartbeat=0,error=""}
local s=enzui_sidebar
s.saved_sizes=s.saved_sizes or {}
s.pin_scope=s.pin_scope or "shared"
local function active_desktop()
    local ws=s.output and hl.get_active_workspace(s.output)
    return ws and ws.id
end
local function in_scope(e,id,scope)
    return (scope or s.pin_scope)=="shared" or (type(id)=="number" and e.desktop==id)
end
local function visible_entries()
    local id=active_desktop();local entries={}
    for _,e in ipairs(s.entries)do if in_scope(e,id) then entries[#entries+1]=e end end
    return entries
end
local remote
local function scene() return s.native and hl.plugin.enzui_sidebar_scene or nil end
local shell_classes={
    ["dev.enzui.Desktop"]=true,["dev.enzui.ControlCenter"]=true,["dev.enzui.WorkspaceOverview"]=true,
    ["dev.enzui.Sidebar"]=true,["dev.enzui.SidebarPane"]=true,["dev.enzui.SidebarBacking"]=true,["dev.enzui.Launcher"]=true,["dev.enzui.QuickSearch"]=true,
    ["dev.enzui.RemotePlaceholder"]=true,["dev.enzui.PluginPanel"]=true,["dev.enzui.AssistantPanel"]=true,["dev.enzui.AssistantTrigger"]=true,
    ["dev.enzui.NotificationCenter"]=true,["dev.enzui.NotificationToasts"]=true,
}
local function emit() hl.dispatch(hl.dsp.event("enzui-sidebar")) end
local function floating() return s.metrics and s.metrics.islands~=nil end
local function visible_error() return s.fullscreen and "" or s.error end
local function adding()
    return floating() and s.reveal and #s.entries<16 and not (s.drag and s.drag.entry) and not (s.pending and s.pending.repin)
end
local function checked(action)
    local result=hl.dispatch(action)
    assert(result and result.ok==true,"Sidebar window operation failed")
end
local function focus_preserving_pointer(window)
    local point=hl.get_cursor_pos()
    local no_warps=hl.get_config("cursor.no_warps")
    local follow=hl.get_config("input.follow_mouse")
    assert(type(no_warps)=="boolean" and type(follow)=="number" and follow%1==0 and follow>=0 and follow<=3,"Invalid sidebar pointer policy")
    hl.config({cursor={no_warps=true},input={follow_mouse=3}})
    local ok,error=pcall(function()
        checked(hl.dsp.focus({window=window}))
        checked(hl.dsp.cursor.move({x=point.x,y=point.y}))
    end)
    hl.config({cursor={no_warps=no_warps},input={follow_mouse=follow}})
    assert(ok,error)
    local current=hl.get_cursor_pos()
    assert(current.x==point.x and current.y==point.y,"Sidebar focus moved the pointer")
end
local function monitor()
    for _,m in ipairs(hl.get_monitors()) do
        if m.name==s.output and m.enabled and not m.is_mirror then return m end
    end
end
local function read_size_limits(w)
    local api=scene()
    if api and api.size_limits then return api.size_limits(w.address,s.owner) end
end
local function fit_pane(w,h,max_w,max_h,limits)
    if limits then
        local upper_w,upper_h=math.min(max_w,limits.max_w),math.min(max_h,limits.max_h)
        if limits.min_w<=upper_w and limits.min_h<=upper_h then
            return math.min(math.max(w,limits.min_w),upper_w),math.min(math.max(h,limits.min_h),upper_h),true
        end
    end
    return math.min(w,max_w),math.min(h,max_h),not limits
end
local function geometry(entry,anchor)
    local m=monitor();if not m then return end
    local w,h=enzui_output_w(m),enzui_output_h(m)
    if w<320 or h<240 then return end
    local metrics=s.metrics or enzui_resolve_metrics()
    local entries=visible_entries()
    local has_error=not entry and visible_error()~=""
    local desired={x=360,y=640}
    if not entry then
        anchor=s.anchor
        for _,item in ipairs(entries)do
            if item.id==s.positioned then entry=item;break end
        end
    end
    if entry then desired=entry.pending_size or entry.pinned_size or s.saved_sizes[entry.stable_id] or desired end
    local limits=entry and entry.size_limits
    local desired_w=type(desired.x)=="number" and desired.x>=80 and desired.x<=8192 and desired.x%1==0 and desired.x or 360
    local desired_h=type(desired.y)=="number" and desired.y>=80 and desired.y<=8192 and desired.y%1==0 and desired.y or 640
    if metrics.islands then
        local style=metrics.islands
        local top=s.bar=="top" and enzui_bar_band(metrics) or style.edge_gap
        local bottom=h-(s.bar=="bottom" and enzui_bar_band(metrics) or style.edge_gap)
        local available=bottom-top
        local slot=metrics.rail-4
        local count=math.max(1,#entries+(has_error and 1 or 0)+(adding() and #entries>0 and 1 or 0))
        local rail_h=math.min(available,#entries==0 and not has_error and 120 or 16+count*slot+(count-1)*6)
        local rail_y=math.floor(m.y+top+(available-rail_h)/2+0.5)
        local rail_x=s.side=="left" and m.x+style.side_gap or m.x+w-style.side_gap-metrics.rail
        local inset=math.max(6,math.ceil(style.radius*(1-math.sqrt(0.5))))
        local aw,ah,fits=fit_pane(desired_w,desired_h,
            math.min(1536-2*inset,w-2*style.side_gap-metrics.rail-8-2*inset),
            math.min(1024-2*inset,available-2*inset),limits)
        if aw<1 or ah<1 or rail_h<1 then return end
        local index=1
        for i,e in ipairs(entries)do if entry and e.id==entry.id then index=i;break end end
        local group_top=8+math.max(0,(rail_h-16-count*slot-(count-1)*6)/2)
        local anchor=anchor or (rail_y+group_top+(index-1)*(slot+6)+slot/2)
        local ay=math.floor(math.max(m.y+top+inset,math.min(anchor-ah/2,m.y+bottom-inset-ah))+0.5)
        local pane_x=s.side=="left" and rail_x+metrics.rail+8 or rail_x-8-aw-2*inset
        return {monitor=m,rail_x=rail_x,rail_y=rail_y,rail_w=metrics.rail,rail_h=rail_h,
            surface_x=rail_x,surface_w=metrics.rail,canvas_y=m.y+top,canvas_h=available,app_x=pane_x+inset,app_y=ay,app_w=aw,app_h=ah,
            pane_x=pane_x,pane_y=ay-inset,pane_w=aw+2*inset,pane_h=ah+2*inset,pane_inset=inset,
            icon_x=rail_x+metrics.rail/2,icon_y=anchor,fits=fits}
    end
    local top=s.bar=="top" and metrics.bar or metrics.frame
    local aw,ah,fits=fit_pane(desired_w,desired_h,math.min(1536-12,w-2*metrics.rail-12),
        math.min(1024-12,h-metrics.bar-metrics.frame-12),limits)
    local index=1
    for i,e in ipairs(entries) do if entry and e.id==entry.id then index=i;break end end
    local slot=metrics.rail-4
    local count=math.max(1,#entries+(has_error and 1 or 0))
    local rail_h=h-metrics.bar-metrics.frame
    local group_top=8+math.max(0,(rail_h-16-count*slot-(count-1)*6)/2)
    local anchor=anchor or (m.y+top+group_top+(index-1)*(slot+6)+slot/2)
    local bottom=m.y+h-(s.bar=="bottom" and metrics.bar or metrics.frame)
    local ay=math.max(m.y+top+6,math.min(anchor-ah/2,bottom-6-ah))
    local ax=s.side=="left" and m.x+metrics.rail+6 or m.x+w-metrics.rail-6-aw
    if s.native then ax=math.floor(ax+0.5);ay=math.floor(ay+0.5) end
    return {monitor=m,rail_x=s.side=="left" and m.x or m.x+w-metrics.rail,rail_y=m.y+top,
        rail_w=metrics.rail,rail_h=rail_h,surface_x=s.side=="left" and m.x or m.x+w-metrics.rail-metrics.radius,surface_w=metrics.rail+metrics.radius,app_x=ax,app_y=ay,app_w=aw,app_h=ah,
        pane_x=s.side=="left" and m.x+metrics.rail or m.x+w-metrics.rail-12-aw,pane_y=ay-6,pane_w=aw+12,pane_h=ah+12,pane_inset=6,fits=fits}
end
local backing_hidden
local function hide_backing_in_fullscreen(hidden)
    if backing_hidden==hidden then return end
    hl.window_rule({
        name="sidebar-fullscreen-backing",
        match={class="^dev[.]enzui[.]Sidebar(Backing|Pane)$"},
        enabled=hidden,opacity="0 override 0 override",no_focus=true,
    })
    backing_hidden=hidden
end
local function update_visibility(g)
    if s.enabled==false then
        s.autohidden=true;s.visibility_state=nil;s.keyboard_reveal=nil;s.fullscreen=false
        hide_backing_in_fullscreen(false)
        return
    end
    local api=scene()
    if not api or not api.rail_context then
        if s.autohidden then s.autohidden=false;emit() end
        s.visibility_state=nil;s.keyboard_reveal=nil;s.fullscreen=false
        hide_backing_in_fullscreen(false)
        return
    end
    local behavior=floating() and s.behavior or {visibility="reserved"}
    behavior=behavior or {visibility="reserved"}
    local context=api.rail_context(s.owner)
    assert(context and type(context.now)=="number" and type(context.held)=="boolean" and type(context.keyboard)=="boolean","Sidebar visibility context unavailable")
    if s.keyboard_reveal=="active" and not context.keyboard then s.keyboard_reveal=nil end
    if s.keyboard_reveal=="waiting" and context.keyboard then s.keyboard_reveal="active" end
    local point=hl.get_cursor_pos();local m=g.monitor;local workspace=hl.get_active_workspace(s.output)
    local covered,fullscreen=false,false
    for _,w in ipairs(hl.get_windows())do
        if w.mapped and not w.hidden and w.pid~=s.owner and not shell_classes[w.class] and w.monitor and w.monitor.name==s.output and not remote(w)
            and w.workspace and type(w.workspace.id)=="number" and (w.pinned or (workspace and w.workspace.id==workspace.id)) then
            fullscreen=fullscreen or w.fullscreen==2
            if behavior.visibility=="fullscreen" then covered=covered or w.fullscreen==2
            elseif behavior.visibility=="overlap" then
                covered=covered or enzui_sidebar_capsule_overlaps({x=g.rail_x,y=g.rail_y,width=g.rail_w,height=g.rail_h},{x=w.at.x,y=w.at.y,width=w.size.x,height=w.size.y})
            end
        end
    end
    local previous_error=visible_error()
    s.fullscreen=fullscreen
    local in_output=point.x>=m.x and point.x<m.x+enzui_output_w(m) and point.y>=(g.canvas_y or m.y) and point.y<(g.canvas_y or m.y)+(g.canvas_h or enzui_output_h(m))
    local edge=in_output and (s.side=="left" and point.x<m.x+3 or s.side=="right" and point.x>=m.x+enzui_output_w(m)-3)
    local body=in_output and point.y>=g.rail_y-12 and point.y<g.rail_y+g.rail_h+12 and
        (s.side=="left" and point.x<g.rail_x+g.rail_w+8 or s.side=="right" and point.x>=g.rail_x-8)
    local entries=visible_entries()
    local occupied=s.reveal or #entries>0 or s.error~=""
    local shown
    for _,e in ipairs(entries) do
        if e.id==s.shown then shown=e;break end
    end
    local pane_open=shown~=nil and not (s.after_close and s.after_close.park)
    local explicit=s.keyboard_reveal~=nil or pane_open
    hide_backing_in_fullscreen(fullscreen and not pane_open)
    s.visibility_state=enzui_sidebar_visibility_step(s.visibility_state,{now=context.now,occupied=occupied,covered=covered,fullscreen=fullscreen,edge_hover=edge,body_hover=body,held=context.held,
        force=explicit or (not fullscreen and (s.reveal or s.shown~=nil or s.error~=""))})
    local hidden=occupied and not s.visibility_state.visible
    if s.autohidden~=hidden or visible_error()~=previous_error then s.autohidden=hidden;emit() end
end
function enzui_sidebar_reservation_inputs()
    if s.enabled==false then return nil,false end
    return s.behavior,#visible_entries()>0 or s.error~=""
end
local function update_reservations(g)
    local api=scene();if not api or not api.fullscreen_area then return end
    -- Only a living shell owner may reserve its output band.
    if not api.owner_alive(s.owner) then return end
    local behavior,occupied=enzui_sidebar_reservation_inputs()
    local reserved,fullscreen=enzui_sidebar_reservations(s.metrics,s.bar,s.side,behavior,occupied)
    enzui_apply_reservations(g.monitor,reserved)
    local enabled=s.enabled~=false and floating() and s.behavior~=nil
    local key=table.concat({tostring(enabled),fullscreen.left,fullscreen.top,fullscreen.right,fullscreen.bottom,g.monitor.width,g.monitor.height,g.monitor.scale},":")
    if key~=s.viewport_key then
        assert(api.fullscreen_area(s.owner,enabled,fullscreen.left,fullscreen.top,fullscreen.right,fullscreen.bottom),"Fullscreen work area rejected")
        s.viewport_key=key
    end
end
local function sync_rail_input()
    local current=geometry()
    -- Output removal invalidates scene calls; pointer callbacks must quietly stop.
    if not monitor() then return end
    if current then update_visibility(current);update_reservations(current) end
    local visible=s.enabled~=false and not s.autohidden and (s.reveal or #visible_entries()>0 or s.error~="")
    local api=scene()
    if api and api.tiling_mode then
        local tiling=false
        if enzui_desktop_tiling_at then tiling=enzui_desktop_tiling_at(active_desktop())==true end
        -- Restated only on a change: the provider paints the output for each.
        if s.native_tiling~=tiling then
            assert(api.tiling_mode(s.owner,tiling),"Native tiling mode rejected")
            s.native_tiling=tiling
        end
    end
    if api and (s.native_version or 0)>=7 then
        local g=geometry()
        if g then
            local metrics=s.metrics or enzui_resolve_metrics()
            if metrics.islands then
                assert(s.native_version>=8 and api.rail(s.owner,visible,g.surface_x,g.canvas_y,g.surface_w,g.canvas_h,g.rail_w,0,s.side=="right",true,true),"Floating sidebar rail rejected")
            else
                assert(api.rail(s.owner,visible,g.surface_x,g.rail_y,g.surface_w,g.rail_h,g.rail_w,metrics.frame,s.side=="right"),"Native sidebar rail rejected")
            end
        end
    end
    local rail
    for _,w in ipairs(hl.get_windows()) do
        if w.mapped and w.pid==s.owner and w.class=="dev.enzui.Sidebar" and w.monitor and w.monitor.name==s.output then
            assert(not rail,"Ambiguous sidebar rail")
            rail=w
        end
    end
    if not rail then return end
    local marker="enzui-sidebar-chrome-"..s.owner
    local tagged=false
    for _,tag in ipairs(rail.tags or {}) do if tag==marker then tagged=true;break end end
    if not tagged then checked(hl.dsp.window.tag({window=rail,tag="+"..marker})) end
    if s.rail_input~=visible or s.rail_input_owner~=s.owner then
        hl.window_rule({name="sidebar-rail-input",match={class="^dev[.]enzui[.]Sidebar$",tag=marker},no_focus=not visible})
        s.rail_input=visible;s.rail_input_owner=s.owner
    end
end
local function place_chrome(g)
    sync_rail_input()
    for _,w in ipairs(hl.get_windows())do
        if w.mapped and w.pid==s.owner and w.monitor and w.monitor.name==s.output then
            local x,y
            if w.class=="dev.enzui.Sidebar" then x,y=g.surface_x,g.canvas_y or g.rail_y
            elseif w.class=="dev.enzui.SidebarPane" or w.class=="dev.enzui.SidebarBacking" then
                x,y=g.pane_x,g.pane_y
                -- The compositor sizes the backing: a floating window's own
                -- resize request is not applied, and a pane resized with its
                -- app would otherwise keep the old backing and never reopen.
                if w.size.x~=g.pane_w or w.size.y~=g.pane_h then checked(hl.dsp.window.resize({window=w,x=g.pane_w,y=g.pane_h})) end
            end
            if x and (w.at.x~=x or w.at.y~=y) then checked(hl.dsp.window.move({window=w,x=x,y=y})) end
            if s.reveal and w.class=="dev.enzui.Sidebar" then checked(hl.dsp.window.alter_zorder({window=w,mode="top"})) end
        end
    end
end
remote=function(w)
    for _,tag in ipairs(w.tags or {}) do if tag:match("^enzremote%-") then return true end end
    return w.monitor and w.monitor.name:match("^ENZREMOTE%-")~=nil
end
local function known(e)
    local ok,value=pcall(function()return e.window.mapped and e.window.stable_id==e.stable_id and e.window.pid==e.pid end)
    return ok and value
end
local function owned(e)
    return (known(e) and e.window.monitor and e.window.monitor.name==s.output and not remote(e.window))==true
end
local function window_conflict(w)
    local api=scene()
    if api and api.window_conflict then return api.window_conflict(w.address,s.owner) end
    for _,other in ipairs(hl.get_windows()) do
        if other.mapped and other.pid==w.pid and other.address~=w.address then return true end
    end
    return false
end
local function eligible(w)
    return w.mapped and w.pid and w.pid>1 and w.stable_id and not w.hidden and not shell_classes[w.class] and w.monitor and w.monitor.name==s.output
        and w.workspace and type(w.workspace.id)=="number" and not w.pinned and w.fullscreen==0 and not w.group and not remote(w)
end
local function find(id)
    for index,e in ipairs(s.entries) do if e.id==id then return e,index end end
end
local function original(w)
    return {at=w.at,size=w.size,workspace=w.workspace.id,floating=w.floating}
end
local function remove_sidebar_tags(e)
    checked(hl.dsp.window.tag({window=e.window,tag="-enzui-sidebar-"..e.id}))
    checked(hl.dsp.window.tag({window=e.window,tag="-enzui-sidebar-app"}))
end
local function restore(e,detached)
    assert(owned(e),"Sidebar app is unavailable")
    local target=hl.get_active_workspace(s.output)
    assert(target and type(target.id)=="number","No desktop for sidebar restoration")
    local function tiling_at(id) return enzui_desktop_tiling_at~=nil and enzui_desktop_tiling_at(id)==true end
    local drop=tiling_at(target.id) and (e.drop_point or (detached and hl.get_cursor_pos()))
    local workspace=target.id
    if not drop then
        for _,ws in ipairs(hl.get_workspaces()) do
            if ws.id==e.original.workspace and ws.monitor and ws.monitor.name==s.output then workspace=ws.id;break end
        end
    end
    local tiling=tiling_at(workspace)
    local at=e.icon_drop or (detached and e.window.at or e.original.at)
    if detached or e.icon_drop then
        local m=monitor();assert(m,"Sidebar output unavailable")
        local metrics=s.metrics or enzui_resolve_metrics()
        local top=s.bar=="top" and enzui_bar_band(metrics) or metrics.frame
        local bottom=s.bar=="bottom" and enzui_bar_band(metrics) or metrics.frame
        at={x=math.max(m.x+metrics.frame,math.min(at.x,m.x+enzui_output_w(m)-e.original.size.x-metrics.frame)),
            y=math.max(m.y+top,math.min(at.y,m.y+enzui_output_h(m)-bottom-e.original.size.y))}
    end
    if scene() then
        assert(scene().restore(e.window.address,s.owner,at.x,at.y,e.original.size.x,e.original.size.y),"Native restoration rejected")
        e.restoring=true;s.restoring=e.id;s.after_close=nil
    end
    checked(hl.dsp.window.pin({window=e.window,action="off"}))
    checked(hl.dsp.window.float({window=e.window,action="on"}))
    checked(hl.dsp.window.move({window=e.window,workspace=workspace,follow=false}))
    checked(hl.dsp.window.resize({window=e.window,x=e.original.size.x,y=e.original.size.y}))
    checked(hl.dsp.window.move({window=e.window,x=at.x,y=at.y}))
    remove_sidebar_tags(e)
    if tiling and scene() and scene().place_tile then
        local point=drop or {x=at.x+e.original.size.x/2,y=at.y+e.original.size.y/2}
        assert(scene().tiling_mode(s.owner,true),"Native tiling mode rejected")
        assert(scene().place_tile(e.window.address,s.owner,point.x,point.y),"Native tile placement rejected")
    elseif not e.original.floating and tiling then
        checked(hl.dsp.window.float({window=e.window,action="off"}))
    end
    if (s.native_version or 0)>=2 then
        assert(scene().relocated(e.window.address,s.owner),"Native relocation confirmation rejected")
    end
    if e.pinned_size then s.saved_sizes[e.stable_id]=e.pinned_size end
end
function enzui_sidebar_unpin(id,detached)
    local e,index=find(id);if not e then return {ok=true} end
    if scene() then
        if e.restoring then return {ok=true} end
        local phase=scene().status(e.window.address)
        if phase=="dragging" then return {ok=true} end
        if phase=="detached" then detached=true end
        if s.shown==id and phase~="closed" and phase~="failed" and phase~="detached" and phase~="dialog" then
            s.after_close={id=id,unpin=true,detached=detached}
            if phase~="closing" and not scene().hide(e.window.address) then s.error="restore-needed";emit();return {ok=false} end
            emit();return {ok=true}
        end
    end
    local ok=pcall(restore,e,detached)
    if not ok then e.restore_pending=true;s.error="restore-needed";emit();return {ok=false} end
    if scene() then
        e.restore_pending=nil;emit();return {ok=true}
    end
    if s.shown==id then s.shown=nil end
    table.remove(s.entries,index);s.error="";emit();return {ok=true}
end
function enzui_sidebar_icon_feedback(id,owner,light,x,y)
    local e=find(id);local api=scene()
    if owner==s.owner and e and owned(e) and not e.restoring and api and api.icon_feedback then api.icon_feedback(owner,light,e.window.address,x,y) end
    return {ok=true}
end
function enzui_sidebar_cancel_icon_feedback(owner)
    local api=scene()
    if owner==s.owner and api and api.cancel_icon_drop then api.cancel_icon_drop(owner) end
    return {ok=true}
end
function enzui_sidebar_restore_icon(id,owner,x,y)
    assert(owner==s.owner,"Sidebar icon owner changed")
    assert(type(x)=="number" and x%1==0 and math.abs(x)<=100000 and type(y)=="number" and y%1==0 and math.abs(y)<=100000,"Invalid sidebar icon drop")
    local e=find(id);local api=scene();local g=geometry()
    if not e or not owned(e) or e.restoring or window_conflict(e.window) or not api or not api.icon_drop or not g then return {ok=false} end
    if not api.icon_drop(owner,x,y) then return {ok=true} end
    local m=g.monitor
    if x<m.x or y<m.y or x>=m.x+enzui_output_w(m) or y>=m.y+enzui_output_h(m) then return {ok=true} end
    e.drop_point={x=x,y=y}
    e.icon_drop={x=math.floor(x-e.original.size.x/2+0.5),y=y-24}
    if s.shown and s.shown~=id then
        if not enzui_sidebar_hide().ok then e.icon_drop=nil;return {ok=false} end
        s.next_restore=id;emit();return {ok=true}
    end
    return enzui_sidebar_unpin(id,false)
end
function enzui_sidebar_hide()
    if not s.shown then return {ok=true} end
    local e=find(s.shown)
    if not e or not known(e) then s.shown=nil;emit();return {ok=true} end
    if not owned(e) then s.error="unavailable";emit();return {ok=false} end
    if window_conflict(e.window) then s.error="multi-window";emit();return {ok=false} end
    if scene() then
        s.next_show=nil
        if e.restoring then return {ok=true} end
        local phase=scene().status(e.window.address)
        if phase=="failed" then return enzui_sidebar_unpin(e.id,false) end
        if phase=="dragging" or phase=="detached" then return enzui_sidebar_unpin(e.id,true) end
        if s.after_close and s.after_close.id==e.id and (phase=="closing" or phase=="closed") then
            return {ok=true}
        end
        e.focused=nil;s.after_close={id=e.id,park=true}
        if not scene().hide(e.window.address) then s.error="restore-needed";emit();return {ok=false} end
        emit();return {ok=true}
    end
    local ok=pcall(function()
        checked(hl.dsp.window.pin({window=e.window,action="off"}))
        checked(hl.dsp.window.move({window=e.window,workspace="special:enzui-miniapps",follow=false}))
    end)
    if not ok then s.error="restore-needed";emit();return {ok=false} end
    s.shown=nil;s.error="";emit();return {ok=true}
end
function enzui_sidebar_toggle(id,anchor)
    if s.enabled==false then return {ok=false} end
    local e=find(id)
    if e and not in_scope(e,active_desktop()) then return {ok=false} end
    if not e or not owned(e) then s.error="unavailable";emit();return {ok=false} end
    if e.restore_pending then return enzui_sidebar_unpin(id,false) end
    assert(anchor==nil or (type(anchor)=="number" and anchor%1==0 and math.abs(anchor)<=100000),"Invalid sidebar icon anchor")
    if s.shown==id then
        if scene() and s.after_close and s.after_close.park then
            if not scene().show(e.window.address) then return {ok=false} end
            s.after_close=nil;s.next_show=nil;emit();return {ok=true}
        end
        return enzui_sidebar_hide()
    end
    if not enzui_sidebar_hide().ok then return {ok=false} end
    if scene() and (s.shown or (s.restoring and find(s.restoring))) then
        s.next_show={id=id,anchor=anchor};return {ok=true}
    end
    if window_conflict(e.window) then
        e.return_error="desktop-return";return enzui_sidebar_unpin(id,false)
    end
    e.size_limits=read_size_limits(e.window)
    local g=geometry(e,anchor);local ws=hl.get_active_workspace(s.output)
    if not g or not ws or type(ws.id)~="number" or ws.id<=0 then return {ok=false} end
    if not g.fits then s.error="size-limits";emit();return {ok=false} end
    s.anchor=anchor;s.positioned=id
    local ok=pcall(function()
        if scene() then
            assert(scene().prepare(e.window.address,s.owner,g.app_x,g.app_y,g.app_w,g.app_h,s.side=="right",g.pane_inset,g.icon_x,g.icon_y),"Native preparation rejected")
        end
        for _,chrome in ipairs(hl.get_windows()) do
            if chrome.mapped and (chrome.class=="dev.enzui.SidebarPane" or chrome.class=="dev.enzui.SidebarBacking") and chrome.pid==s.owner
                and chrome.monitor and chrome.monitor.name==s.output then
                checked(hl.dsp.window.move({window=chrome,x=g.pane_x,y=g.pane_y}))
                if scene() then checked(hl.dsp.window.alter_zorder({window=chrome,mode="bottom"})) end
            end
        end
        checked(hl.dsp.window.float({window=e.window,action="on"}))
        checked(hl.dsp.window.resize({window=e.window,x=g.app_w,y=g.app_h}))
        checked(hl.dsp.window.move({window=e.window,workspace=ws.id,follow=false}))
        checked(hl.dsp.window.move({window=e.window,x=g.app_x,y=g.app_y}))
        checked(hl.dsp.window.pin({window=e.window,action="on"}))
        if not scene() then focus_preserving_pointer(e.window) end
    end)
    if not ok then e.restore_pending=true;s.error="restore-needed";emit();return {ok=false} end
    e.focused=nil;s.shown=id;s.error="";emit();return {ok=true}
end
function enzui_sidebar_pin_window(w,before)
    if s.enabled==false or #s.entries>=16 or not eligible(w) or window_conflict(w)
        or (scene() and not scene().supports(w.address,s.owner)) then
        s.error="ineligible";emit();return {ok=false}
    end
    for _,e in ipairs(s.entries) do if known(e) and e.window.address==w.address then return {ok=true} end end
    local e={window=w,id=s.next_id,pid=w.pid,stable_id=w.stable_id,original=before or original(w),desktop=w.workspace.id,pinned_size=s.saved_sizes[w.stable_id]}
    e.size_limits=read_size_limits(w)
    local g=geometry(e)
    if g and not g.fits then s.error="size-limits";emit();return {ok=false} end
    s.next_id=s.next_id+1;s.entries[#s.entries+1]=e
    local ok=pcall(function()
        checked(hl.dsp.window.tag({window=w,tag="+enzui-sidebar-"..e.id}))
        checked(hl.dsp.window.tag({window=w,tag="+enzui-sidebar-app"}))
        checked(hl.dsp.window.float({window=w,action="on"}))
        checked(hl.dsp.window.move({window=w,workspace="special:enzui-miniapps",follow=false}))
    end)
    if not ok then
        e.restore_pending=true;s.error="restore-needed"
        if pcall(restore,e,false) and not scene() then table.remove(s.entries,#s.entries) end
        emit();return {ok=false}
    end
    s.error="";emit();return {ok=true}
end
local function inside(point,at,size)
    return point.x>=at.x and point.y>=at.y and point.x<at.x+size.x and point.y<at.y+size.y
end
local function at_edge(point,g)
    return point.y>=g.rail_y and point.y<g.rail_y+g.rail_h and point.x>=g.rail_x and point.x<g.rail_x+g.rail_w
end
local function pocket_hint(show,hover)
    local api=scene();if not api or (s.native_version or 0)<6 then return end
    local g=geometry()
    if floating() then
        local next_hover=show and hover or false
        if s.drop_hover~=next_hover then s.drop_hover=next_hover;emit() end
        return
    end
    if not g then api.pocket(s.owner,false,false,0,0,0,0,false);return end
    if show then
        if s.anchor==nil then s.anchor=g.app_y+g.app_h/2 end
        for _,w in ipairs(hl.get_windows()) do
            if w.mapped and w.pid==s.owner and w.class=="dev.enzui.SidebarBacking" and w.monitor and w.monitor.name==s.output then
                checked(hl.dsp.window.alter_zorder({window=w,mode="bottom"}))
            end
        end
    end
    api.pocket(s.owner,show,hover,g.pane_x,g.pane_y,g.pane_w,g.pane_h,s.side=="right")
end
local function hold_pocket()
    if floating() then return end
    local api=scene()
    if api and (s.native_version or 0)>=6 then api.hold_pocket(s.owner) end
end
local function at_drop(point,g)
    if floating() then
        local m=g.monitor
        local top=m.y+(s.bar=="top" and enzui_bar_band(s.metrics) or s.metrics.islands.edge_gap)
        local bottom=m.y+enzui_output_h(m)-(s.bar=="bottom" and enzui_bar_band(s.metrics) or s.metrics.islands.edge_gap)
        local left=s.side=="left" and m.x or g.rail_x-24
        local right=s.side=="left" and g.rail_x+g.rail_w+24 or m.x+enzui_output_w(m)
        return point.y>=top and point.y<bottom and point.x>=left and point.x<right
    end
    if at_edge(point,g) then return true end
    if not s.reveal or (s.native_version or 0)<6 then return false end
    local width=g.pane_w*0.36
    local x=s.side=="left" and g.pane_x or g.pane_x+g.pane_w-width
    return inside(point,{x=x,y=g.pane_y},{x=width,y=g.pane_h})
end
local function repin_entry(id)
    local e=find(id);local api=scene()
    if not e or not owned(e) or not api or (s.native_version or 0)<6 then return false end
    local phase=api.status(e.window.address)
    if phase=="dragging" then return false,true end
    if phase~="detached" then return false end
    if window_conflict(e.window) then e.return_error="desktop-return";return enzui_sidebar_unpin(id,true).ok end
    if e.resize_pending then
        local size=e.released_size or e.window.size
        if size and type(size.x)=="number" and type(size.y)=="number" and size.x>=80 and size.x<=8192 and size.y>=80 and size.y<=8192
            and size.x%1==0 and size.y%1==0 then e.pending_size={x=size.x,y=size.y} end
    end
    e.size_limits=read_size_limits(e.window)
    local g=geometry();if not g then return false end
    if not g.fits then e.return_error="size-limits";return enzui_sidebar_unpin(id,true).ok end
    local ok=pcall(function()
        assert(api.repin(e.window.address,s.owner,g.app_x,g.app_y,g.app_w,g.app_h,s.side=="right",g.pane_inset),"Native repin rejected")
        if e.window.size.x~=g.app_w or e.window.size.y~=g.app_h then
            checked(hl.dsp.window.resize({window=e.window,x=g.app_w,y=g.app_h}))
        end
        checked(hl.dsp.window.move({window=e.window,x=g.app_x,y=g.app_y}))
        if not e.window.pinned then checked(hl.dsp.window.pin({window=e.window,action="on"})) end
    end)
    if not ok then e.restore_pending=true;s.error="restore-needed";emit();return false end
    if e.pending_size then e.pinned_size={x=g.app_w,y=g.app_h};s.saved_sizes[e.stable_id]=e.pinned_size;e.pending_size=nil end
    e.resize_pending=nil;e.released_size=nil
    e.focused=nil;e.restore_pending=nil;s.shown=id;s.after_close=nil;s.error="";emit();return true
end
function enzui_sidebar_press()
    if not s.output or s.enabled==false then return end
    if s.pending and s.pending.window then s.pending=nil end
    local point=hl.get_cursor_pos();local g=geometry();if not g then return end
    if s.shown then
        local e=find(s.shown)
        if e and owned(e) and inside(point,e.window.at,e.window.size) then
            -- The grip sits in the pane's outer bottom corner: bottom right beside a
            -- left-hand rail, bottom left beside a right-hand one (where the scene draws it).
            local grip=s.side=="right" and e.window.at.x+6 or e.window.at.x+e.window.size.x-28
            local resizing=e.window.size.y>=160 and point.x>=grip
                and point.x<grip+22
                and point.y>=e.window.at.y+e.window.size.y-28
                and point.y<e.window.at.y+e.window.size.y-6
            s.drag={entry=e.id,before=original(e.window),point=point,resizing=resizing};return
        end
        if at_edge(point,g) then return end
        enzui_sidebar_hide()
    end
    local candidates={}
    for _,w in ipairs(hl.get_windows()) do
        if eligible(w) and inside(point,w.at,w.size) then candidates[#candidates+1]={window=w,before=original(w),stable_id=w.stable_id,pid=w.pid} end
    end
    s.drag={candidates=candidates,point=point}
end
local function moved(w,before)
    local at,size=w.at,w.size
    return size.x==before.size.x and size.y==before.size.y and math.abs(at.x-before.at.x)+math.abs(at.y-before.at.y)>=12
end
local function candidate_drag(candidate,released)
    local w=candidate.window
    if not eligible(w) or w.stable_id~=candidate.stable_id or w.pid~=candidate.pid then return false end
    local api=scene()
    if api and api.drag_active and api.drag_drop then
        return released and api.drag_drop(s.owner,w.address) or (not released and api.drag_active(s.owner,w.address))
    end
    return candidate.before.floating and moved(w,candidate.before)
end
function enzui_sidebar_release()
    if s.enabled==false then s.pending=nil;s.drag=nil;return end
    local drag=s.drag;s.drag=nil
    if drag then
        if drag.entry then
            local e=find(drag.entry)
            if e and owned(e) then
                local phase=scene() and scene().status(e.window.address)
                local detached=(s.native_version or 0)>=6 and (phase=="dragging" or phase=="detached")
                if detached or moved(e.window,drag.before) then
                    if detached and drag.resizing then
                        e.resize_pending=true
                        local size=e.window.size
                        if size and type(size.x)=="number" and type(size.y)=="number" and size.x>=80 and size.x<=8192
                            and size.y>=80 and size.y<=8192 and size.x%1==0 and size.y%1==0 then
                            e.released_size={x=size.x,y=size.y}
                        end
                    end
                    local g=geometry()
                    if detached and g and (at_drop(hl.get_cursor_pos(),g) or not moved(e.window,drag.before)) then
                        s.pending={repin=e.id}
                    else e.drop_point=hl.get_cursor_pos();s.pending={unpin=e.id} end
                end
            end
        else
            local g=geometry()
            if g and at_drop(hl.get_cursor_pos(),g) then
                s.pending={candidates=drag.candidates or {},point=hl.get_cursor_pos()}
            end
        end
    end
    if s.reveal and not (s.pending and (s.pending.candidates or s.pending.repin)) then s.reveal=false;emit() end
    if s.pending and s.pending.repin then hold_pocket() end
    pocket_hint(false,false)
end
local function native_tick()
    local api=scene();if not api then return end
    if (s.native_version or 0)>=2 and not api.owner_alive(s.owner) then
        s.next_show=nil
        if not s.restoring then
            for _,e in ipairs(s.entries) do
                if owned(e) then
                    enzui_sidebar_unpin(e.id,false)
                    if s.restoring then break end
                end
            end
        end
    end
    for index=#s.entries,1,-1 do
        local e=s.entries[index]
        if known(e) then
            local phase=api.status(e.window.address)
            if phase=="foreign" then
                api.finish(e.window.address)
                e.restoring=nil;e.restore_pending=true
                if s.restoring==e.id then s.restoring=nil end
                if s.shown==e.id then s.shown=nil;s.after_close=nil end
                s.error="unavailable";emit()
            elseif e.restoring then
                if phase=="restored" then
                    assert(api.finish(e.window.address),"Native restoration release rejected")
                    local cleaned=pcall(remove_sidebar_tags,e)
                    if s.shown==e.id then s.shown=nil end
                    if s.restoring==e.id then s.restoring=nil end
                    if cleaned then
                        table.remove(s.entries,index);s.error=e.return_error or ""
                    else
                        e.restoring=nil;e.restore_pending=true;s.error="restore-needed"
                    end
                    emit()
                elseif phase=="failed" then
                    local ok,released=pcall(api.finish,e.window.address)
                    if ok and released and s.shown==e.id then s.shown=nil end
                    e.restoring=nil;e.restore_pending=true;s.restoring=nil;s.error="restore-needed";emit()
                end
            elseif s.shown==e.id then
                if phase=="detached" then
                    if not (s.pending and s.pending.repin==e.id) and not (s.drag and s.drag.entry==e.id) then enzui_sidebar_unpin(e.id,true) end
                elseif phase=="dialog" then
                    e.return_error="desktop-return"
                    enzui_sidebar_unpin(e.id,false)
                elseif phase=="open" and not e.focused and owned(e) then
                    focus_preserving_pointer(e.window);e.focused=true
                elseif phase=="closed" and s.after_close and s.after_close.id==e.id then
                    local after=s.after_close
                    if after.unpin then
                        enzui_sidebar_unpin(e.id,after.detached)
                    elseif after.park then
                        local ok=pcall(function()
                            assert(owned(e),"Sidebar ownership changed")
                            checked(hl.dsp.window.pin({window=e.window,action="off"}))
                            checked(hl.dsp.window.move({window=e.window,workspace="special:enzui-miniapps",follow=false}))
                            assert(api.finish(e.window.address),"Native close release rejected")
                        end)
                        if ok then s.shown=nil;s.after_close=nil;s.error=""
                        else e.restore_pending=true;s.error="restore-needed" end
                        emit()
                    end
                elseif phase=="failed" and not e.restore_pending then
                    e.restore_pending=true;s.error="restore-needed";emit()
                end
            end
        end
    end
    if not s.shown and not s.restoring and s.next_restore then
        local id=s.next_restore;s.next_restore=nil
        enzui_sidebar_unpin(id,false)
    elseif not s.shown and not s.restoring and s.next_show then
        local next=s.next_show;s.next_show=nil
        enzui_sidebar_toggle(next.id,next.anchor)
    end
end
-- Poll rapidly only during sidebar motion; activity entry points wake idle polling.
-- At rest the tick is a safety net: the compositor events that change what
-- it checks (desktops, windows closing or opening, outputs) wake it at once.
local FAST_TICK,IDLE_TICK=40,2000
local function sidebar_busy()
    return s.drag~=nil or s.pending~=nil or s.shown~=nil or s.restoring~=nil or s.next_show~=nil
        or s.next_restore~=nil or s.after_close~=nil or s.reveal==true or s.keyboard_reveal~=nil
end
function enzui_sidebar_wake()
    if enzui_sidebar_timer and s.tick_ms~=FAST_TICK then s.tick_ms=FAST_TICK;enzui_sidebar_timer:set_timeout(FAST_TICK) end
end
local tick_once
function enzui_sidebar_tick()
    tick_once()
    local next=sidebar_busy() and FAST_TICK or IDLE_TICK
    s.tick_ms=next
    if enzui_sidebar_timer then enzui_sidebar_timer:set_timeout(next) end
end
tick_once=function()
    if not s.output then return end
    local desktop=active_desktop()
    if type(desktop)=="number" and desktop~=s.last_desktop then
        s.last_desktop=desktop
        if s.pin_scope=="desktop" and s.shown then
            local e=find(s.shown)
            if e and not in_scope(e,desktop) then
                s.next_show=nil
                enzui_sidebar_hide()
            end
        end
        emit()
    end
    if scene() then
        local ok=pcall(native_tick)
        if not ok then s.error="restore-needed";emit() end
    end
    local chrome=geometry();if chrome then pcall(place_chrome,chrome) end
    if s.keyboard_reveal=="waiting" and scene() and scene().rail_state(s.owner).input then
        for _,w in ipairs(hl.get_windows())do
            if w.mapped and w.pid==s.owner and w.class=="dev.enzui.Sidebar" and w.monitor and w.monitor.name==s.output then
                focus_preserving_pointer(w);break
            end
        end
    end
    local changed=false
    for index=#s.entries,1,-1 do
        local e=s.entries[index]
        if not known(e) then
            if scene() then pcall(scene().finish,"") end
            if s.shown==e.id then s.shown=nil;s.after_close=nil end
            if s.restoring==e.id then s.restoring=nil end
            table.remove(s.entries,index);changed=true
        end
    end
    if changed then emit() end
    if s.enabled==false then
        s.pending=nil;s.next_show=nil;s.next_restore=nil
        if s.shown then
            enzui_sidebar_hide()
        elseif not s.restoring then
            for _,e in ipairs(s.entries) do
                if not e.restoring then
                    if not enzui_sidebar_unpin(e.id,false).ok then s.enabled=true;emit() end
                    break
                end
            end
        end
        sync_rail_input()
        return
    end
    if #s.entries>0 then
        -- A shell that is gone leaves its pinned apps to be restored. The
        -- native scene knows whether the shell process lives; without it, the
        -- shell counts as gone when it has not read the sidebar for longer
        -- than its slowest safety-net read (30 s).
        local api=scene()
        local gone
        if api and (s.native_version or 0)>=2 then
            gone=not api.owner_alive(s.owner)
        else
            s.heartbeat=s.heartbeat+(s.tick_ms or FAST_TICK)
            gone=s.heartbeat>45000
        end
        if gone then
            for index=#s.entries,1,-1 do enzui_sidebar_unpin(s.entries[index].id,false) end
            s.heartbeat=0
        end
    end
    local pending=s.pending;s.pending=nil
    if pending then
        if pending.repin then
            local _,waiting=repin_entry(pending.repin)
            if waiting then s.pending=pending;pocket_hint(true,false);return end
        elseif pending.unpin then enzui_sidebar_unpin(pending.unpin,true)
        elseif pending.candidates then
            local point=hl.get_cursor_pos();local g=geometry()
            if g and at_drop(point,g) and math.abs(point.x-pending.point.x)+math.abs(point.y-pending.point.y)<=2 then
                for _,candidate in ipairs(pending.candidates) do
                    local ok,result=pcall(candidate_drag,candidate,true)
                    if ok and result then
                        local api=scene()
                        if api and not api.supports(candidate.window.address,s.owner) then s.pending={window=candidate,attempts=0}
                        else enzui_sidebar_pin_window(candidate.window,candidate.before) end
                        break
                    end
                end
            end
        elseif pending.window then
            local candidate=pending.window;local w=candidate.window;local api=scene()
            local ok,valid=pcall(function()return eligible(w) and w.stable_id==candidate.stable_id and w.pid==candidate.pid and not window_conflict(w) end)
            if ok and valid then
                if api and not api.supports(w.address,s.owner) and pending.attempts<25 then
                    pending.attempts=pending.attempts+1;s.pending=pending
                else enzui_sidebar_pin_window(w,candidate.before) end
            end
        end
        if not s.pending then
            if s.reveal then s.reveal=false;emit() end
            pocket_hint(false,false)
        end
    end
    local drag=s.drag
    if not drag then return end
    if drag.entry then
        local e=find(drag.entry)
        local phase=e and owned(e) and scene() and scene().status(e.window.address)
        if phase=="dragging" and drag.resizing and e then
            local size=e.window.size
            if size and type(size.x)=="number" and type(size.y)=="number" and size.x>=80 and size.x<=8192
                and size.y>=80 and size.y<=8192 and size.x%1==0 and size.y%1==0
                and (not e.pending_size or e.pending_size.x~=size.x or e.pending_size.y~=size.y) then
                e.pending_size={x=size.x,y=size.y};s.positioned=e.id;emit()
            end
        end
        local g=geometry()
        if (s.native_version or 0)>=6 and phase=="dragging" and g then
            if not s.reveal then s.reveal=true;emit() end
            pocket_hint(true,at_drop(hl.get_cursor_pos(),g))
        else
            if s.reveal then s.reveal=false;emit() end
            pocket_hint(false,false)
            if not e or not owned(e) then s.drag=nil end
        end
        return
    end
    local g=geometry()
    local reveal=false
    if g and (floating() or at_drop(hl.get_cursor_pos(),g)) then
        for _,candidate in ipairs(drag.candidates or {}) do
            local ok,result=pcall(candidate_drag,candidate,false)
            if ok and result then reveal=true;break end
        end
    end
    if reveal~=s.reveal then s.reveal=reveal;emit() end
    pocket_hint(reveal,reveal and g and at_drop(hl.get_cursor_pos(),g))
end
function enzui_sidebar_configure(output,side,bar,owner,metrics,motion,behavior,pin_scope,enabled)
    if enabled==nil then enabled=true end
    assert(type(enabled)=="boolean","Invalid sidebar enabled state")
    metrics=enzui_resolve_metrics(metrics)
    pin_scope=pin_scope or "shared"
    assert(pin_scope=="shared" or pin_scope=="desktop","Invalid sidebar pin scope")
    local api=hl.plugin and hl.plugin.enzui_sidebar_scene
    if metrics.islands then
        assert(api and api.version()==8 and api.supports_islands and api.supports_islands(),"Floating sidebar needs the prepared Islands compositor")
    end
    if behavior then
        assert(type(behavior)=="table" and (behavior.visibility=="fullscreen" or behavior.visibility=="overlap" or behavior.visibility=="reserved") and type(behavior.fullscreen_padding)=="number" and behavior.fullscreen_padding%1==0 and behavior.fullscreen_padding>=0 and behavior.fullscreen_padding<=32,"Invalid sidebar behavior")
        assert(api and api.rail_context and api.fullscreen_area,"Sidebar behavior needs a current prepared compositor")
    end
    motion=motion or {rate=100,windows=true}
    assert(type(motion)=="table" and type(motion.rate)=="number" and motion.rate%1==0 and motion.rate>=25 and motion.rate<=200 and type(motion.windows)=="boolean","Invalid sidebar motion preference")
    assert(type(output)=="string" and output:match("^[%w._-]+$") and not output:match("^ENZREMOTE%-"),"Explicit sidebar output required")
    assert(side=="left" or side=="right","Invalid sidebar edge")
    assert(bar=="top" or bar=="bottom","Invalid bar edge")
    local target
    for _,m in ipairs(hl.get_monitors()) do if m.name==output and m.enabled and not m.is_mirror then target=m end end
    assert(target and enzui_output_w(target)>=320 and enzui_output_h(target)>=240,"Sidebar output unavailable")
    local resized=false
    for _,key in ipairs({"frame","bar","rail","icon"})do if not s.metrics or s.metrics[key]~=metrics[key] then resized=true;break end end
    local previous=s.metrics and s.metrics.islands
    if (previous~=nil)~=(metrics.islands~=nil) then resized=true
    elseif previous then
        for key,value in pairs(metrics.islands)do if previous[key]~=value then resized=true;break end end
    end
    local shown_entry=s.shown and find(s.shown)
    local hide_for_scope=shown_entry and pin_scope=="desktop" and not in_scope(shown_entry,active_desktop(),pin_scope)
    if s.output and (s.owner~=owner or s.output~=output or s.side~=side or s.bar~=bar or resized or hide_for_scope) then
        if not enzui_sidebar_hide().ok then return {ok=false} end
    end
    if scene() and (s.shown or s.restoring) and (s.owner~=owner or s.output~=output or s.side~=side or s.bar~=bar or resized or hide_for_scope) then
        return {ok=false}
    end
    if api and (api.version()==1 or api.version()==2 or api.version()==3 or api.version()==4 or api.version()==5 or api.version()==6 or api.version()==7 or api.version()==8) then
        if not s.native and #s.entries>0 then return {ok=false} end
        assert(api.configure(owner,output),"Native sidebar owner rejected")
        if api.version()>=4 then
            assert(api.motion(owner,motion.rate,motion.windows),"Native sidebar motion rejected")
            local applied=api.motion_state(owner)
            assert(type(applied)=="table" and applied.rate==motion.rate and applied.windows==motion.windows,"Native sidebar motion was not applied")
        end
        s.native=true;s.native_version=api.version()
    elseif s.native then
        return {ok=false}
    end
    if s.owner~=owner then enzui_sidebar_cancel_drag() end
    if s.bar~=bar or s.side~=side or resized then s.anchor=nil end
    s.output=output;s.side=side;s.bar=bar;s.owner=owner;s.heartbeat=0;s.metrics=metrics;s.rail_input=nil;s.behavior=behavior;s.viewport_key=nil;s.native_tiling=nil;s.pin_scope=pin_scope;s.enabled=enabled
    if not enabled then
        s.reveal=false;s.keyboard_reveal=nil;s.pending=nil;s.next_show=nil
        if s.shown and not enzui_sidebar_hide().ok then return {ok=false} end
    end
    hl.window_rule({name="sidebar-app-chrome",match={tag="enzui-sidebar-app"},border_size=0,no_shadow=true,no_anim=true,no_xdg_drags=true,decorate=false})
    for _,e in ipairs(s.entries)do
        if owned(e) and not e.restoring then
            local tagged=false
            for _,tag in ipairs(e.window.tags or {})do if tag=="enzui-sidebar-app"then tagged=true;break end end
            if not tagged then checked(hl.dsp.window.tag({window=e.window,tag="+enzui-sidebar-app"})) end
        end
    end
    assert(geometry(),"Sidebar output unavailable")
    emit();return {ok=true}
end
function enzui_sidebar_snapshot()
    sync_rail_input()
    s.heartbeat=0
    local g=geometry();assert(g,"Sidebar output unavailable")
    local rows={}
    for _,e in ipairs(visible_entries()) do
        local ok,class=pcall(function()return e.window.class end)
        class=(ok and type(class)=="string" and class or ""):gsub("[^%w%._%-]",""):sub(1,128)
        rows[#rows+1]=string.format('{"id":%d,"pid":%d,"available":%s,"class":"%s"}',e.id,e.pid,tostring(owned(e)),class)
    end
    local canvas=g.canvas_y and string.format(',"canvas":[%.0f,%.0f,%.0f,%.0f]',g.surface_x,g.canvas_y,g.surface_w,g.canvas_h) or ""
    canvas=canvas..',"enabled":'..tostring(s.enabled~=false)..',"icon_drag":'..tostring(s.enabled~=false and scene() and type(scene().icon_drop)=="function" or false)..',"autohidden":'..tostring(s.autohidden==true)..',"keyboard_reveal":'..tostring(s.keyboard_reveal~=nil)
    return string.format('{"output":"%s","side":"%s","bar":"%s","reveal":%s,"drop_hover":%s,"drop_add":%s%s,"shown":%d,"error":"%s","origin":[%.0f,%.0f],"rail":[%.0f,%.0f,%.0f,%.0f],"pane":[%.0f,%.0f,%.0f,%.0f],"inset":%.0f,"entries":[%s],"native":%s,"scene_version":%d}',
        s.output,s.side,s.bar,tostring(s.reveal),tostring(s.drop_hover==true),tostring(adding()==true),canvas,(s.shown and find(s.shown) and in_scope(find(s.shown),active_desktop()) and s.shown or 0),visible_error(),g.monitor.x,g.monitor.y,g.surface_x,g.rail_y,g.surface_w,g.rail_h,g.pane_x,g.pane_y,g.pane_w,g.pane_h,g.pane_inset or 0,table.concat(rows,","),tostring(s.native==true),s.native_version or 0)
end
function enzui_sidebar_clear_error() s.error="";emit();return {ok=true} end
function enzui_sidebar_keyboard_reveal()
    if #visible_entries()==0 or not scene() or not scene().rail_context then return end
    s.keyboard_reveal="waiting";emit()
end
function enzui_sidebar_cancel_drag()
    s.keyboard_reveal=nil
    local api=scene();if api and api.cancel_icon_drop then api.cancel_icon_drop(s.owner) end
    local drag=s.drag;s.drag=nil;s.pending=nil
    if drag and drag.entry and (s.native_version or 0)>=6 then
        local e=find(drag.entry)
        local phase=e and owned(e) and scene() and scene().status(e.window.address)
        if phase=="dragging" or phase=="detached" then s.pending={repin=e.id} end
    end
    if s.reveal and not s.pending then s.reveal=false;emit() end
    if s.pending and s.pending.repin then hold_pocket() end
    pocket_hint(false,false)
end
function enzui_install_sidebar()
    if not enzui_sidebar_keyboard_binding then
        enzui_sidebar_keyboard_binding=hl.bind("SUPER + SHIFT + B",function()enzui_sidebar_keyboard_reveal()end)
    end
    if not enzui_sidebar_cancel_binding then
        enzui_sidebar_cancel_binding=hl.on("input.keyboard.key",function(key,_,state) if key==9 and state==1 then enzui_sidebar_cancel_drag() end end)
    end
    assert(hl.plugin and hl.plugin.enzui_pointer_events and hl.plugin.enzui_pointer_events.version()==1,"Native pointer event plugin is required")
    if not enzui_sidebar_pointer_binding then
        for _,binding in ipairs(enzui_sidebar_bindings or {}) do binding:remove() end
        enzui_sidebar_bindings={}
        enzui_sidebar_pointer_binding=hl.on("enzui.pointer_button",function(button,state)
            if button==272 then if state==1 then enzui_sidebar_press() else enzui_sidebar_release() end
            elseif button==273 then
                if state==1 and s.shown then
                    enzui_sidebar_press()
                    if s.drag and s.drag.entry then s.drag.resizing=true else s.drag=nil end
                elseif state==0 and s.drag and s.drag.resizing then enzui_sidebar_release() end
            end
        end)
    end
    if not enzui_sidebar_timer then enzui_sidebar_timer=hl.timer(function()enzui_sidebar_tick()end,{timeout=40,type="repeat"}) end
    if not enzui_sidebar_event_bindings then
        enzui_sidebar_event_bindings={}
        for _,name in ipairs({"workspace.active","window.open","window.close","window.destroy","window.move_to_workspace","window.fullscreen","window.minimize","monitor.layout_changed"}) do
            table.insert(enzui_sidebar_event_bindings,hl.on(name,function() enzui_sidebar_wake() end))
        end
    end
    enzui_sidebar_installed=true
end
for _,name in ipairs({"enzui_sidebar_press","enzui_sidebar_release","enzui_sidebar_toggle","enzui_sidebar_pin_window",
        "enzui_sidebar_unpin","enzui_sidebar_hide","enzui_sidebar_keyboard_reveal","enzui_sidebar_configure",
        "enzui_sidebar_restore_icon","enzui_sidebar_cancel_drag"}) do
    local original=_G[name]
    if original then
        _G[name]=function(...)
            local results=table.pack(original(...))
            enzui_sidebar_wake()
            return table.unpack(results,1,results.n)
        end
    end
end
