function enzui_apply_reservations(monitor,reserved)
    assert(monitor and monitor.enabled and not monitor.is_mirror and not monitor.name:match("^ENZREMOTE%-"),"Unsupported reservation output")
    enzui_shell_reservation_base=enzui_shell_reservation_base or {}
    enzui_shell_reservation_base[monitor.name]={left=reserved.left,right=reserved.right,top=reserved.top,bottom=reserved.bottom}
    local dock=enzui_dock_reservation
    if dock and dock.amount>0 then
        local alive=false
        for _,window in ipairs(hl.get_windows()) do
            if window.mapped and window.class=="dev.enzui.Dock" and window.pid==dock.owner then alive=true;break end
        end
        if not alive then dock.amount=0 end
    end
    if dock and dock.output==monitor.name and dock.amount>0 then
        reserved={left=reserved.left,right=reserved.right,top=reserved.top,bottom=reserved.bottom}
        reserved[dock.side]=reserved[dock.side]+dock.amount
    end
    local changed=false
    for _,key in ipairs({"left","right","top","bottom"})do
        local value=reserved[key]
        assert(type(value)=="number" and value==value and value>=0 and value<=512,"Invalid shell reservation")
        if not monitor.reserved or monitor.reserved[key]~=value then changed=true end
    end
    if not changed then return false end
    assert(type(monitor.refresh_rate)=="number" and monitor.refresh_rate>0 and monitor.refresh_rate<1000 and monitor.scale>0,"Output mode unavailable")
    hl.monitor({output=monitor.name,mode=string.format("%dx%d@%.6f",monitor.width,monitor.height,monitor.refresh_rate),
        position=string.format("%.0fx%.0f",monitor.x,monitor.y),scale=monitor.scale,transform=monitor.transform,reserved=reserved})
    return true
end
