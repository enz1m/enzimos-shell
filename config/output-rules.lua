-- Persist output-management changes as monitor rules; transient client state is lost on rule
-- refresh.
function enzui_pin_outputs()
    enzui_pinned_outputs=enzui_pinned_outputs or {}
    for _,m in ipairs(hl.get_monitors()) do
        if not m.name:match("^ENZREMOTE%-") and not m.is_mirror then
            local mode=string.format("%dx%d@%.6f",m.width,m.height,m.refresh_rate or 0)
            local position=string.format("%.0fx%.0f",m.x,m.y)
            local key=m.enabled and table.concat({mode,position,tostring(m.scale),tostring(m.transform)},"|") or "off"
            if enzui_pinned_outputs[m.name]~=key then
                if not m.enabled then
                    hl.monitor({output=m.name,disabled=true})
                elseif type(m.refresh_rate)=="number" and m.refresh_rate>0 and m.scale>0 then
                    hl.monitor({output=m.name,mode=mode,position=position,scale=m.scale,transform=m.transform,disabled=false})
                end
                enzui_pinned_outputs[m.name]=key
            end
        end
    end
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
