-- Window border colours: the chosen tone at the chosen opacities. Automatic keeps a quiet
-- grey edge on floating windows and gives the focused tile the accent, where focus is the
-- main thing to see.
function enzui_window_border_colors()
    local p=enzui_window_borders or {color=0,custom=0x0a84ff,active=80,inactive=20}
    local function alpha(percent) return string.format("%02x",math.floor(percent*255/100+0.5)) end
    local accent=type(enzui_drag_maximize_accent)=="string" and enzui_drag_maximize_accent:match("^%x%x%x%x%x%x$") and enzui_drag_maximize_accent or nil
    local custom=p.color==4 and string.format("%06x",p.custom or 0x0a84ff) or nil
    local tone=(p.color==1 and accent) or (p.color==3 and "ffffff") or custom or "92969d"
    local active="rgba("..tone..alpha(p.active)..")"
    local inactive="rgba(55585d"..alpha(p.inactive)..")"
    local tiled=(p.color==0 and accent) and "rgba("..accent..alpha(p.active)..")" or active
    return active,inactive,tiled
end
local function tiled_border_rule(tiled,inactive)
    if hl.window_rule then
        pcall(hl.window_rule,{name="enzui-tiled-focus",match={float=false},border_color=tiled.." "..inactive})
    end
end
-- Applies the border colours alone, when the accent changes.
function enzui_apply_window_borders()
    local active,inactive,tiled=enzui_window_border_colors()
    hl.config({general={col={active_border=active,inactive_border=inactive}}})
    tiled_border_rule(tiled,inactive)
end
function enzui_apply_desktop_preferences(inner,outer,border,rounding,animations,focus_pointer,natural,pointer_speed,animation_rate,device)
    local function integer(value,min,max)
        assert(type(value)=="number" and value%1==0 and value>=min and value<=max,"Invalid desktop preference")
    end
    integer(inner,0,48);integer(outer,0,96);integer(border,0,6);integer(rounding,0,32);integer(pointer_speed,-100,100)
    integer(animation_rate,25,200)
    assert(type(animations)=="boolean" and type(focus_pointer)=="boolean" and type(natural)=="boolean","Invalid desktop switch")
    assert(type(device)=="table","Invalid input preferences")
    integer(device.scroll,25,300);integer(device.repeat_delay,150,1000);integer(device.repeat_rate,10,80)
    integer(device.zoom,100,800);integer(device.cursor,16,96)
    integer(device.border_color,0,4);integer(device.border_custom,0,0xffffff);integer(device.border_active,0,100);integer(device.border_inactive,0,100)
    for _,key in ipairs({"accel","left_handed","tap","dwt","gestures"}) do assert(type(device[key])=="boolean","Invalid input switch") end
    assert(type(device.kb_options)=="string" and device.kb_options:match("^[a-z0-9:_,]*$"),"Invalid keyboard option")
    assert(type(device.kb_variant)=="string" and device.kb_variant:match("^[a-z0-9_,-]*$"),"Invalid keyboard variant")
    assert(type(device.x11_sharp)=="boolean","Invalid X11 scaling")
    assert(type(device.terminal)=="string" and device.terminal:match("^[%w%./_+][%w%./_+-]*$"),"Invalid terminal")
    assert(type(device.kb_layout)=="string" and device.kb_layout:match("^[a-z0-9_,]*$"),"Invalid keyboard layout")
    local focus=focus_pointer and 1 or 2
    local sensitivity=pointer_speed/100
    local scroll=device.scroll/100
    local accel=device.accel and "adaptive" or "flat"
    -- Preserve XKB options outside Settings-owned Caps Lock and layout-switch choices.
    local options={}
    for option in string.gmatch(hl.get_config("input.kb_options") or "","[^,]+") do
        if not option:match("^caps:") and not option:match("^grp:") then options[#options+1]=option end
    end
    for option in string.gmatch(device.kb_options,"[^,]+") do
        local seen=false
        for _,existing in ipairs(options) do seen=seen or existing==option end
        if not seen then options[#options+1]=option end
    end
    local kb_options=table.concat(options,",")
    local input={follow_mouse=focus,float_switch_override_focus=0,sensitivity=sensitivity,natural_scroll=natural,
        accel_profile=accel,left_handed=device.left_handed,scroll_factor=scroll,
        repeat_delay=device.repeat_delay,repeat_rate=device.repeat_rate,kb_options=kb_options,
        touchpad={natural_scroll=natural,tap_to_click=device.tap,disable_while_typing=device.dwt,scroll_factor=scroll}}
    if device.kb_layout~="" then input.kb_layout=device.kb_layout;input.kb_variant=device.kb_variant end
    hl.env("ENZUI_TERMINAL",device.terminal)
    hl.env("XCURSOR_SIZE",tostring(device.cursor));hl.env("HYPRCURSOR_SIZE",tostring(device.cursor))
    enzui_window_borders={color=device.border_color,custom=device.border_custom,active=device.border_active,inactive=device.border_inactive}
    local border_active,border_inactive,border_tiled=enzui_window_border_colors()
    hl.config({
        cursor={zoom_factor=device.zoom/100},
        xwayland={force_zero_scaling=device.x11_sharp},
        general={gaps_in=inner,gaps_out=outer,float_gaps=outer,border_size=border,
            col={active_border=border_active,inactive_border=border_inactive}},
        decoration={rounding=rounding},animations={enabled=animations},
        input=input,
    })
    tiled_border_rule(border_tiled,border_inactive)
    -- A compositor config reload clears both the gesture bindings and this Lua state.
    if device.gestures and not enzui_trackpad_gestures then
        hl.gesture({fingers=3,direction="horizontal",action="workspace"})
        hl.gesture({fingers=3,direction="up",action=function() hl.dispatch(hl.dsp.event("enzui-command:overview")) end})
        enzui_trackpad_gestures=true
    elseif not device.gestures and enzui_trackpad_gestures then
        hl.gesture({fingers=3,direction="horizontal",action="unset"})
        hl.gesture({fingers=3,direction="up",action="unset"})
        enzui_trackpad_gestures=false
    end
    local duration=300/animation_rate
    hl.animation({leaf="global",enabled=true,speed=duration,bezier="desktop-ease"})
    hl.animation({leaf="workspaces",enabled=true,speed=duration,bezier="desktop-ease",style="slide"})
    local function gap(key,value)
        local actual=hl.get_config(key)
        assert(type(actual)=="table" and actual.top==value and actual.right==value and actual.bottom==value and actual.left==value,"Gap setting was not applied")
    end
    gap("general.gaps_in",inner);gap("general.gaps_out",outer);gap("general.float_gaps",outer)
    assert(hl.get_config("general.border_size")==border and hl.get_config("decoration.rounding")==rounding,"Window decoration settings were not applied")
    assert(hl.get_config("animations.enabled")==animations and hl.get_config("input.follow_mouse")==focus and hl.get_config("input.float_switch_override_focus")==0,"Window behavior settings were not applied")
    assert(math.abs(hl.get_config("input.sensitivity")-sensitivity)<0.001 and hl.get_config("input.natural_scroll")==natural and hl.get_config("input.touchpad.natural_scroll")==natural,"Pointer settings were not applied")
    assert(hl.get_config("input.accel_profile")==accel and hl.get_config("input.left_handed")==device.left_handed and math.abs(hl.get_config("input.scroll_factor")-scroll)<0.001,"Mouse settings were not applied")
    assert(hl.get_config("input.touchpad.tap_to_click")==device.tap and hl.get_config("input.touchpad.disable_while_typing")==device.dwt,"Touchpad settings were not applied")
    assert(hl.get_config("input.repeat_delay")==device.repeat_delay and hl.get_config("input.repeat_rate")==device.repeat_rate and hl.get_config("input.kb_options")==kb_options,"Keyboard settings were not applied")
    assert(device.kb_layout=="" or hl.get_config("input.kb_layout")==device.kb_layout,"Keyboard layout was not applied")
    assert(math.abs(hl.get_config("cursor.zoom_factor")-device.zoom/100)<0.001,"Zoom was not applied")
    return true
end
