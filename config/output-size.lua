-- Output logical dimensions include scale and quarter-turn rotation.
function enzui_output_w(m) return (m.transform%2==1 and m.height or m.width)/m.scale end
function enzui_output_h(m) return (m.transform%2==1 and m.width or m.height)/m.scale end

function enzui_app_work_area(m, reserved)
    local outer=hl.get_config and hl.get_config("general.gaps_out") or 12
    local floating=hl.get_config and hl.get_config("general.float_gaps") or -1
    local border=hl.get_config and hl.get_config("general.border_size") or 2
    local function gap(edge)
        local value=type(floating)=="table" and floating[edge] or floating
        if type(value)~="number" or value<0 then value=type(outer)=="table" and outer[edge] or outer end
        return math.max(0,tonumber(value) or 12)+math.max(0,tonumber(border) or 2)
    end
    local left,right,top,bottom=gap("left"),gap("right"),gap("top"),gap("bottom")
    return (m.x or 0)+reserved.left+left,(m.y or 0)+reserved.top+top,
        enzui_output_w(m)-reserved.left-reserved.right-left-right,
        enzui_output_h(m)-reserved.top-reserved.bottom-top-bottom
end
