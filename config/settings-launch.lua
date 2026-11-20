function enzui_settings_command(command,output)
    assert(type(command)=="string" and #command>0 and #command<=8192,"Invalid Settings command")
    assert(type(output)=="string" and output~="" and not output:match("^ENZREMOTE%-"),"Explicit desktop output required")
    local monitor
    for _,candidate in ipairs(hl.get_monitors()) do
        if candidate.name==output and candidate.enabled then monitor=candidate;break end
    end
    assert(monitor and not monitor.is_mirror,"Unsupported Settings output")
    assert(type(monitor.scale)=="number" and monitor.scale>0,"Invalid Settings scale")
    local reserved=monitor.reserved
    assert(type(reserved)=="table","Missing Settings work area")
    for _,edge in ipairs({"left","right","top","bottom"}) do
        assert(type(reserved[edge])=="number" and reserved[edge]>=0 and reserved[edge]<=16384,"Invalid Settings reservation")
    end
    local logical_width,logical_height=enzui_output_w(monitor),enzui_output_h(monitor)
    assert(logical_width>=240 and logical_width<=16384 and logical_height>=240 and logical_height<=16384,"Invalid Settings output geometry")
    local _,_,available_width,available_height=enzui_app_work_area(monitor,reserved)
    local width=math.floor(math.min(1024,available_width))
    local height=math.floor(math.min(768,available_height))
    assert(width>=240 and height>=240,"Settings work area is too small")
    return string.format("env ENZUI_SETTINGS_WIDTH=%d ENZUI_SETTINGS_HEIGHT=%d ",width,height)..command
end
