enzui_switcher = enzui_switcher or {revision=0, active=false, entries={}, selected=1}

local shell_classes = {
    ["dev.enzui.Desktop"]=true, ["dev.enzui.Dock"]=true, ["dev.enzui.Launcher"]=true, ["dev.enzui.QuickSearch"]=true,
    ["dev.enzui.ControlCenter"]=true, ["dev.enzui.WorkspaceOverview"]=true,
    ["dev.enzui.Sidebar"]=true, ["dev.enzui.SidebarPane"]=true, ["dev.enzui.SidebarBacking"]=true,
    ["dev.enzui.PluginPanel"]=true, ["dev.enzui.WindowSpread"]=true, ["dev.enzui.Switcher"]=true,
    ["dev.enzui.AssistantPanel"]=true, ["dev.enzui.AssistantTrigger"]=true,
    ["dev.enzui.Dictation"]=true, ["dev.enzui.DictationPermission"]=true,
    ["dev.enzui.NotificationCenter"]=true, ["dev.enzui.NotificationToasts"]=true,
    ["dev.enzui.WritingTools"]=true, ["dev.enzui.EmojiPicker"]=true, ["dev.enzui.Keyboard"]=true, ["dev.enzui.Shortcuts"]=true, ["dev.enzui.Osd"]=true, ["dev.enzui.Capture"]=true,
}
local function eligible(window, workspace, output)
    return window and window.mapped and not window.hidden and window.accepts_input ~= false
        and window.workspace and window.workspace.id == workspace
        and window.monitor and window.monitor.name == output and not output:match("^ENZREMOTE%-")
        and not shell_classes[window.class]
        and not (function()
            for _,tag in ipairs(window.tags or {}) do
                if tag:match("^enzui%-sidebar%-") or tag:match("^enzremote%-") then return true end
            end
            return false
        end)()
end
local function valid(entry, state)
    local ok, value = pcall(function()
        return eligible(entry.window, state.workspace, state.output)
            and entry.window.stable_id == entry.stable_id and entry.window.pid == entry.pid
    end)
    return ok and value
end
local function changed()
    enzui_switcher.revision = enzui_switcher.revision + 1
    hl.dispatch(hl.dsp.event("enzui-command:switch-state"))
end
function enzui_switcher_step(direction, same_app)
    assert(direction == 1 or direction == -1, "Invalid switcher direction")
    local state = enzui_switcher
    if not state.active then
        local workspace = hl.get_active_workspace()
        if not workspace or not workspace.monitor or workspace.monitor.name:match("^ENZREMOTE%-") then return end
        state.workspace, state.output = workspace.id, workspace.monitor.name
        state.entries, state.selected, state.same_app = {}, 1, same_app == true
        local active = hl.get_active_window()
        local class = active and active.class
        for _,window in ipairs(hl.get_windows()) do
            if eligible(window, state.workspace, state.output) and (not state.same_app or window.class == class) then
                local rank = window.focus_history_id or 2147483647
                if rank < 0 then rank = 2147483647 end
                state.entries[#state.entries+1] = {window=window, stable_id=window.stable_id, pid=window.pid, rank=rank}
            end
        end
        table.sort(state.entries, function(a,b)
            if a.rank == b.rank then return a.stable_id < b.stable_id end
            return a.rank < b.rank
        end)
        while #state.entries > 64 do table.remove(state.entries) end
        state.active = #state.entries > 0
    end
    if state.active then state.selected = ((state.selected - 1 + direction) % #state.entries) + 1 end
    changed()
end
function enzui_switcher_cancel()
    enzui_switcher.active = false
    enzui_switcher.entries = {}
    changed()
end
function enzui_switcher_commit()
    local state = enzui_switcher
    if not state.active then return end
    local entry = state.entries[state.selected]
    state.active = false
    local workspace = hl.get_active_workspace()
    if entry and workspace and workspace.id == state.workspace and workspace.monitor
        and workspace.monitor.name == state.output and valid(entry, state) then
        local warps = hl.get_config("cursor.no_warps")
        hl.config({cursor={no_warps=true}})
        local ok, result = pcall(function()
            local raised = hl.dispatch(hl.dsp.window.alter_zorder({window=entry.window,mode="top"}))
            if not raised or raised.ok ~= true then return raised or {ok=false} end
            return hl.dispatch(hl.dsp.focus({window=entry.window}))
        end)
        hl.config({cursor={no_warps=warps}})
        if not ok or result and result.ok == false then
            state.entries = {}
            changed()
            if not ok then error(result) end
            return result
        end
    end
    state.entries = {}
    changed()
end
function enzui_switcher_pick(revision, index)
    local state = enzui_switcher
    if state.revision ~= revision or not state.active or type(index) ~= "number" or index%1 ~= 0 or index < 1 or index > #state.entries then
        return "stale"
    end
    state.selected = index
    enzui_switcher_commit()
    return "ok"
end
function enzui_switcher_snapshot()
    local state = enzui_switcher
    local rows = {}
    if state.active then
        for _,entry in ipairs(state.entries) do
            if not valid(entry, state) then enzui_switcher_cancel(); return enzui_switcher_snapshot() end
            rows[#rows+1] = string.format('"%s"',entry.stable_id)
        end
    end
    return string.format('{"revision":%d,"active":%s,"selected":%d,"entries":[%s]}',
        state.revision,tostring(state.active),state.selected,table.concat(rows,","))
end
