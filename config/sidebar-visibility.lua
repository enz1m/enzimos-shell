function enzui_sidebar_visibility_step(previous,context)
    assert(type(context)=="table" and type(context.now)=="number" and context.now==context.now and context.now>=0 and context.now<math.huge,"Invalid sidebar clock")
    for _,key in ipairs({"occupied","covered","edge_hover","body_hover","held","force","fullscreen"})do
        assert(type(context[key])=="boolean","Invalid sidebar visibility context")
    end
    previous=previous or {visible=false}
    local next={visible=previous.visible==true,at=context.now}
    if previous.at and context.now<previous.at then next.visible=false;return next end
    if not context.occupied then next.visible=false;return next end
    if context.force then next.visible=true;return next end
    if context.fullscreen then next.visible=false;return next end
    if not context.covered then next.visible=true;return next end
    if next.visible then
        if context.edge_hover or context.body_hover or context.held then return next end
        next.hide_at=previous.hide_at or context.now+0.25
        if context.now>=next.hide_at then next.visible=false;next.hide_at=nil end
    elseif context.edge_hover and not context.held then
        next.hover_at=previous.hover_at or context.now
        if context.now-next.hover_at>=0.15 then next.visible=true;next.hover_at=nil end
    end
    return next
end

function enzui_sidebar_capsule_overlaps(rail,window)
    for _,box in ipairs({rail,window})do
        assert(type(box)=="table" and type(box.x)=="number" and type(box.y)=="number" and type(box.width)=="number" and type(box.height)=="number","Invalid overlap rectangle")
        assert(box.x==box.x and box.y==box.y and math.abs(box.x)<math.huge and math.abs(box.y)<math.huge and box.width>0 and box.width<math.huge and box.height>0 and box.height<math.huge,"Invalid overlap rectangle")
    end
    local radius=math.min(rail.width,rail.height)/2
    local left,right=rail.x+radius,rail.x+rail.width-radius
    local top,bottom=rail.y+radius,rail.y+rail.height-radius
    local dx=math.max(window.x-right,0,left-window.x-window.width)
    local dy=math.max(window.y-bottom,0,top-window.y-window.height)
    return dx*dx+dy*dy<radius*radius
end

function enzui_sidebar_reservations(metrics,edge,side,preferences,occupied)
    assert(edge=="top" or edge=="bottom","Invalid bar edge")
    assert(side=="left" or side=="right","Invalid sidebar edge")
    local band=enzui_bar_band(metrics)
    local reserved={left=metrics.frame,right=metrics.frame,top=edge=="top" and band or metrics.frame,bottom=edge=="bottom" and band or metrics.frame}
    local fullscreen={left=reserved.left,right=reserved.right,top=reserved.top,bottom=reserved.bottom}
    if metrics.islands and preferences then
        assert(preferences.visibility=="fullscreen" or preferences.visibility=="overlap" or preferences.visibility=="reserved","Invalid sidebar visibility mode")
        local padding=preferences.fullscreen_padding
        assert(type(padding)=="number" and padding%1==0 and padding>=0 and padding<=32,"Invalid fullscreen padding")
        if occupied and preferences.visibility=="reserved" then reserved[side]=metrics.islands.side_gap+metrics.rail+8 end
        for key,value in pairs(reserved)do fullscreen[key]=math.max(value,padding) end
    elseif not metrics.islands and occupied then
        -- Frame paints the occupied rail as part of the frame: windows (tiled ones included) stop at it.
        reserved[side]=math.max(reserved[side],metrics.rail)
        fullscreen[side]=reserved[side]
    end
    return reserved,fullscreen
end
