-- Persist output-management changes as monitor rules; transient client state is lost on rule
-- refresh.
local function pinned_output(m)
    return not m.name:match("^ENZREMOTE%-") and not m.is_mirror
end

local function output_key(m)
    local mode=string.format("%dx%d@%.6f",m.width,m.height,m.refresh_rate or 0)
    local position=string.format("%.0fx%.0f",m.x,m.y)
    return m.enabled and table.concat({mode,position,tostring(m.scale),tostring(m.transform)},"|") or "off"
end

function enzui_pin_outputs()
    enzui_pinned_outputs=enzui_pinned_outputs or {}
    local changed=false
    for _,m in ipairs(hl.get_monitors()) do
        if pinned_output(m) then
            local mode=string.format("%dx%d@%.6f",m.width,m.height,m.refresh_rate or 0)
            local position=string.format("%.0fx%.0f",m.x,m.y)
            local key=output_key(m)
            local previous=enzui_pinned_outputs[m.name]
            if previous~=key then
                if not m.enabled then
                    hl.monitor({output=m.name,disabled=true})
                elseif type(m.refresh_rate)=="number" and m.refresh_rate>0 and m.scale>0 then
                    hl.monitor({output=m.name,mode=mode,position=position,scale=m.scale,transform=m.transform,disabled=false})
                end
                enzui_pinned_outputs[m.name]=key
                if previous then
                    changed=true
                    if m.enabled and enzui_reflow_output_windows then
                        pcall(enzui_reflow_output_windows,m)
                    end
                end
            end
        end
    end
    -- Mode and scale changes do not emit monitor add/remove IPC events.
    if changed then hl.dispatch(hl.dsp.event("enzui-output-changed")) end
end

-- Defer pinning until output-rule application has settled.
function enzui_install_output_pinning()
    if not enzui_output_pin_binding then
        enzui_output_pin_binding=hl.on("monitor.layout_changed",function()
            if enzui_output_pin_pending then return end
            enzui_output_pin_pending=true
            hl.timer(function()
                enzui_output_pin_pending=false
                pcall(enzui_pin_outputs)
            end,{timeout=400,type="oneshot"})
        end)
    end
    enzui_pin_outputs()
end

-- Whether every output is pinned as it is now, with no pinning due: the shell
-- places its bar after a scale change only then, since pinning moves windows.
function enzui_outputs_settled()
    if enzui_output_pin_pending then return false end
    if not enzui_output_pin_binding then return true end
    local pinned=enzui_pinned_outputs or {}
    for _,m in ipairs(hl.get_monitors()) do
        if pinned_output(m) and pinned[m.name]~=output_key(m) then return false end
    end
    return true
end
