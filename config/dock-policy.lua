function enzui_dock_window(owner)
    local found
    for _,w in ipairs(hl.get_windows()) do
        if w.mapped and w.class=="dev.enzui.Dock" and w.pid==owner then
            assert(not found,"Ambiguous dock surface");found=w
        end
    end
    return found
end
function enzui_dock_place(owner,output,width,height)
    local w=enzui_dock_window(owner)
    if not w then return "pending" end
    local monitor
    for _,m in ipairs(hl.get_monitors()) do if m.name==output then monitor=m end end
    assert(monitor and width>0 and height>0 and width<=16384 and height<=16384,"Dock output unavailable")
    local function checked(action) local result=hl.dispatch(action);assert(result and result.ok,"Dock placement rejected") end
    -- Resizing a floating toplevel preserves its center.
    local resized=w.size.x~=width or w.size.y~=height
    if resized then checked(hl.dsp.window.resize({window=w,x=width,y=height})) end
    if resized or w.at.x~=monitor.x or w.at.y~=monitor.y then
        checked(hl.dsp.window.move({window=w,x=monitor.x,y=monitor.y}))
    end
    checked(hl.dsp.window.alter_zorder({window=w,mode="top"}))
    return "ok"
end
function enzui_dock_target(address,pid,stable)
    local w=hl.get_window("address:"..address)
    if not (w and w.mapped and w.pid==pid and w.stable_id==stable) then return nil end
    for _,tag in ipairs(w.tags or {}) do if tag:match("^enzremote%-guard%-") then return nil end end
    return w
end
function enzui_dock_action(address,pid,stable,action)
    local w=assert(enzui_dock_target(address,pid,stable),"Dock window changed or unavailable")
    enzui_dock_minimized=enzui_dock_minimized or {}
    local previous=hl.get_config("cursor.no_warps")
    assert(type(previous)=="boolean","Missing cursor policy")
    if not previous then hl.config({cursor={no_warps=true}}) end
    local ok,result=pcall(function()
        if action=="close" then return hl.dispatch(hl.dsp.window.close({window=w})) end
        if action=="minimize" then
            assert(w.workspace and type(w.workspace.id)=="number" and w.workspace.id>0,"Window is already hidden")
            for _,tag in ipairs(w.tags or {}) do assert(not tag:match("^enzui%-sidebar%-"),"Miniapp cannot be minimized") end
            enzui_dock_minimized[stable]=w.workspace.id
            return hl.dispatch(hl.dsp.window.move({window=w,workspace="special:enzui-dock-minimized",follow=false}))
        end
        if enzui_dock_minimized[stable] then
            local workspace=enzui_dock_minimized[stable]
            enzui_dock_minimized[stable]=nil
            local moved=hl.dispatch(hl.dsp.window.move({window=w,workspace=workspace,follow=false}))
            assert(moved and moved.ok,"Window could not restore")
        end
        assert(action=="focus","Unknown dock action")
        local focused=hl.dispatch(hl.dsp.focus({window=w}))
        assert(focused and focused.ok,"Window focus rejected")
        return hl.dispatch(hl.dsp.window.alter_zorder({window=w,mode="top"}))
    end)
    if not previous then hl.config({cursor={no_warps=false}}) end
    assert(ok,result);assert(result and result.ok,"Dock action rejected")
    return "ok"
end

function enzui_dock_pointer_inside(w,point)
    local shape=enzui_dock_shape and enzui_dock_shape.owner==w.pid and enzui_dock_shape
    if not shape then return false end
    if shape.menu then return true end
    local x,y=point.x-w.at.x,point.y-w.at.y
    for _,r in ipairs(shape.rectangles) do
        if x>=r[1] and y>=r[2] and x<r[1]+r[3] and y<r[2]+r[4] then return true end
    end
    return false
end
-- Hides the dock while a window on its workspace covers its body.
function enzui_dock_check()
    local shape=enzui_dock_shape
    local dock=shape and shape.body and enzui_dock_window(shape.owner)
    if not dock then return end
    local workspace=hl.get_active_workspace(shape.output)
    local b=shape.body
    local hidden=false
    for _,window in ipairs(hl.get_windows()) do
        local borrowed=false
        for _,tag in ipairs(window.tags or {}) do if tag:match("^enzremote%-guard%-") then borrowed=true end end
        if window.mapped and not window.hidden and not borrowed and window.pid~=shape.owner and window.monitor==dock.monitor and workspace and window.workspace and window.workspace.id==workspace.id then
            local x,y=window.at.x-dock.at.x,window.at.y-dock.at.y
            if x<b[1]+b[3] and x+window.size.x>b[1] and y<b[2]+b[4] and y+window.size.y>b[2] then hidden=true;break end
        end
    end
    if hidden~=shape.overlapped then
        shape.overlapped=hidden;hl.dispatch(hl.dsp.event("enzui-dock-changed"))
    end
end
function enzui_dock_update(owner,output,rectangles,menu,side,amount,body,overlap)
    assert(type(owner)=="number" and type(menu)=="boolean" and type(amount)=="number" and amount>=0 and amount<=160,"Invalid dock policy")
    assert(side=="left" or side=="right" or side=="top" or side=="bottom","Invalid dock edge")
    assert(type(rectangles)=="table" and #rectangles<=3,"Invalid dock regions")
    for _,r in ipairs(rectangles) do assert(#r==4,"Invalid dock rectangle");for _,n in ipairs(r) do assert(type(n)=="number" and n==n and n>=0 and n<=16384,"Invalid dock coordinate") end end
    local w=enzui_dock_window(owner)
    if not w and (#rectangles>0 or menu or amount>0) then return "pending" end
    local old_overlap=enzui_dock_shape and enzui_dock_shape.overlapped
    enzui_dock_shape={owner=owner,output=output,rectangles=rectangles,menu=menu,body=body,overlapped=old_overlap}
    if overlap and not enzui_dock_watch then
        -- Window geometry has no compositor event. A drag or a resize holds a
        -- pointer button, so the overlap is checked every 160 ms only then;
        -- otherwise on the events that can change what covers the dock, a
        -- moment after a key (keyboard moves), and every 2 s for an app that
        -- resizes itself. An idle desktop no longer runs it six times a second.
        local watch={events={}}
        enzui_dock_watch=watch
        local check=enzui_dock_check
        watch.slow=hl.timer(check,{timeout=2000,type="repeat"})
        for _,name in ipairs({"window.open","window.close","window.destroy","window.move_to_workspace","window.fullscreen","window.minimize","workspace.active","monitor.layout_changed"}) do
            table.insert(watch.events,hl.on(name,function() check() end))
        end
        local ok,pointer=pcall(hl.on,"enzui.pointer_button",function(_,state)
            if state==1 and not watch.fast then
                watch.fast=hl.timer(check,{timeout=160,type="repeat"})
            elseif state==0 and watch.fast then
                watch.fast:set_enabled(false);watch.fast=nil;check()
            end
        end)
        if ok then table.insert(watch.events,pointer) else watch.fast=hl.timer(check,{timeout=160,type="repeat"}) end
        table.insert(watch.events,hl.on("input.keyboard.key",function(_,_,state)
            if state==0 and not watch.soon then
                watch.soon=hl.timer(function() watch.soon=nil;check() end,{timeout=250,type="oneshot"})
            end
        end))
        check()
    elseif not overlap and enzui_dock_watch then
        local watch=enzui_dock_watch
        for _,binding in ipairs(watch.events) do binding:remove() end
        for _,key in ipairs({"slow","fast","soon"}) do if watch[key] then watch[key]:set_enabled(false) end end
        enzui_dock_watch=nil
    end
    local monitor
    for _,m in ipairs(hl.get_monitors()) do if m.name==output then monitor=m end end
    if not monitor then return "ok" end
    local previous=enzui_dock_reservation
    if previous and previous.output~=output then
        enzui_dock_reservation=nil
        for _,old in ipairs(hl.get_monitors()) do
            local base=enzui_shell_reservation_base and enzui_shell_reservation_base[old.name]
            if old.name==previous.output and base then enzui_apply_reservations(old,base) end
        end
    end
    local base=enzui_shell_reservation_base and enzui_shell_reservation_base[output]
    if not base then base={left=monitor.reserved.left,right=monitor.reserved.right,top=monitor.reserved.top,bottom=monitor.reserved.bottom} end
    enzui_dock_reservation={owner=owner,output=output,side=side,amount=amount}
    enzui_apply_reservations(monitor,base)
    if w then hl.dispatch(hl.dsp.window.alter_zorder({window=w,mode="top"})) end
    return "ok"
end
