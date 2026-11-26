enzui_workspaces = enzui_workspaces or {}

local shell_classes = {
    ["dev.enzui.Dock"] = true, ["dev.enzui.Desktop"] = true, ["dev.enzui.Launcher"] = true, ["dev.enzui.QuickSearch"] = true,
    ["dev.enzui.ControlCenter"] = true, ["dev.enzui.WorkspaceOverview"] = true,
    ["dev.enzui.Sidebar"] = true, ["dev.enzui.SidebarPane"] = true, ["dev.enzui.SidebarBacking"] = true,
    ["dev.enzui.PluginPanel"] = true, ["dev.enzui.WindowSpread"] = true,
    ["dev.enzui.Shortcuts"] = true,
    ["dev.enzui.AssistantPanel"] = true, ["dev.enzui.AssistantTrigger"] = true,
}
local function valid_id(id)
    return type(id) == "number" and id % 1 == 0 and id > 0 and id < 2147483000
end
function enzui_reconcile_workspaces(previous, active, counts, blocked)
    assert(valid_id(active), "A regular desktop must be active")
    blocked = blocked or {}
    local state = previous or {order={}, ghost=nil, next_id=1}
    local used = state.used or {}
    for id,count in pairs(state.counts or {}) do if count>0 then used[id]=true end end
    for id,count in pairs(counts) do if count>0 then used[id]=true end end
    local ghost = state.ghost
    if ghost and blocked[ghost] then ghost = nil end
    local order, included = {}, {}
    local function append(id)
        assert(valid_id(id), "Invalid desktop identity")
        if not included[id] then table.insert(order,id);included[id]=true end
    end
    for _,id in ipairs(state.order) do
        if id ~= ghost and ((counts[id] or 0)>0 or id==active or not used[id]) and not blocked[id] then append(id) end
    end
    local additions = {}
    for id,count in pairs(counts) do
        assert(valid_id(id) and type(count)=="number" and count>=0 and count%1==0, "Invalid desktop occupancy")
        if count>0 and not included[id] and id~=ghost and not blocked[id] then table.insert(additions,id) end
    end
    table.sort(additions)
    for _,id in ipairs(additions) do append(id) end
    if active~=ghost then append(active) end
    if ghost then append(ghost) end
    if #order==1 and ghost==active and (counts[ghost] or 0)==0 then
        local retained
        for _,id in ipairs(state.order) do
            if id~=ghost and not blocked[id] then retained=id;break end
        end
        if retained then table.insert(order,1,retained);included[retained]=true
        else ghost=nil end
    end
    if not ghost or (counts[ghost] or 0)>0 then
        local candidate=state.next_id
        for _,id in ipairs(order) do candidate=math.max(candidate,id+1) end
        while blocked[candidate] or included[candidate] do candidate=candidate+1 end
        assert(valid_id(candidate), "Desktop identity space exhausted")
        ghost=candidate;state.next_id=candidate+1;append(ghost)
    end
    assert(#order<=128, "Too many desktops")
    state.used={}
    for _,id in ipairs(order) do if used[id] then state.used[id]=true end end
    state.order=order;state.ghost=ghost;state.active=active;state.counts=counts
    return state
end

local function parked(window)
    for _,tag in ipairs(window.tags or {}) do
        if tag:match("^enzremote%-guard%-%x+$") or tag:match("^enzui%-sidebar%-") then return true end
    end
    return false
end
function enzui_desktop_state(output)
    assert(type(output)=="string" and output~="" and not output:match("^ENZREMOTE%-"), "Explicit desktop output required")
    local workspace=hl.get_active_workspace(output)
    assert(workspace and valid_id(workspace.id), "Desktop output is unavailable")
    local counts,blocked={},{}
    for _,ws in ipairs(hl.get_workspaces()) do
        if valid_id(ws.id) and ws.monitor and ws.monitor.name~=output then blocked[ws.id]=true end
    end
    for _,window in ipairs(hl.get_windows()) do
        local ws,mon=window.workspace,window.monitor
        if window.mapped and ws and valid_id(ws.id) and mon and mon.name==output
            and not shell_classes[window.class] and not parked(window) then
            counts[ws.id]=(counts[ws.id] or 0)+1
        end
    end
    local state=enzui_reconcile_workspaces(enzui_workspaces[output],workspace.id,counts,blocked)
    enzui_workspaces[output]=state
    return state
end
function enzui_workspace_snapshot(output)
    local state=enzui_desktop_state(output)
    local rows={}
    for _,id in ipairs(state.order) do
        rows[#rows+1]=string.format('{"id":%d,"windows":%d,"placeholder":%s}',id,state.counts[id] or 0,tostring(id==state.ghost))
    end
    return string.format('{"active":%d,"desktops":[%s]}',state.active,table.concat(rows,","))
end
function enzui_workspace_id_at(output,index)
    assert(type(index)=="number" and index%1==0 and index>=1 and index<=128, "Invalid desktop ordinal")
    return enzui_desktop_state(output).order[index]
end
local function activate(output,id)
    local at=hl.get_cursor_pos and hl.get_cursor_pos()
    local result=hl.dispatch(hl.dsp.focus({workspace=id}))
    if not result or result.ok~=true then return result end
    local target
    for _,window in ipairs(hl.get_windows()) do
        if window.mapped and not window.hidden and window.workspace and window.workspace.id==id
            and window.monitor and window.monitor.name==output
            and not shell_classes[window.class] and not parked(window) then
            local rank=window.focus_history_id or 2147483647
            if rank<0 then rank=2147483647 end
            if not target or rank<target.rank then target={window=window,rank=rank} end
        end
    end
    -- Suppress cursor warps in nested sessions so compositor focus does not diverge from the
    -- parent pointer.
    if target then
        local no_warps=hl.get_config("cursor.no_warps")
        hl.config({cursor={no_warps=true}})
        local ok,focused=pcall(hl.dispatch,hl.dsp.focus({window=target.window}))
        hl.config({cursor={no_warps=no_warps}})
        if not ok then error(focused) end
        result=focused
    end
    -- Restore pointer focus after desktop switching so wheel input stays with the switcher.
    if at and result and result.ok==true then hl.dispatch(hl.dsp.cursor.move({x=at.x,y=at.y})) end
    return result
end
function enzui_install_workspace_keys()
    if enzui_workspace_keys_version==3 then return end
    for index=1,9 do
        local key="SUPER + code:" .. (index + 9)
        hl.unbind(key)
        hl.bind(key,function()
            local workspace=hl.get_active_workspace()
            if not workspace or not workspace.monitor then return end
            local id=enzui_workspace_id_at(workspace.monitor.name,index)
            if id then return activate(workspace.monitor.name,id) end
        end)
    end
    for _, spec in ipairs({{"Prior",-1},{"Next",1}}) do
        local key = "SUPER + " .. spec[1]
        hl.bind(key, function()
            local workspace = hl.get_active_workspace()
            if workspace and workspace.monitor then return enzui_workspace_step(workspace.monitor.name, spec[2]) end
        end)
    end
    enzui_workspace_keys_version=3
end
function enzui_workspace_focus(output,id)
    local state=enzui_desktop_state(output)
    for _,candidate in ipairs(state.order) do
        if candidate==id then return activate(output,id) end
    end
    error("Desktop is no longer available")
end
function enzui_workspace_neighbor(output, current, direction)
    local state = enzui_desktop_state(output)
    for index, id in ipairs(state.order) do
        if id == current then return state.order[index + direction] end
    end
end
function enzui_workspace_step(output,direction)
    assert(direction==1 or direction==-1, "Invalid desktop direction")
    local state=enzui_desktop_state(output)
    for index,id in ipairs(state.order) do
        if id==state.active then
            local next_id=state.order[index+direction]
            if next_id then return activate(output,next_id) end
            return {ok=true}
        end
    end
    error("Active desktop is unavailable")
end
