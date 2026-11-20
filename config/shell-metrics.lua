function enzui_resolve_metrics(input)
    local defaults={frame=8,radius=22,bar=36,rail=36,text=13,icon=16}
    local bounds={frame={0,24},radius={0,32},bar={32,72},rail={32,64},text={10,20},icon={12,24}}
    input=input or defaults
    local result={}
    for key,range in pairs(bounds)do
        local value=input[key]
        assert(type(value)=="number" and value%1==0 and value>=range[1] and value<=range[2],"Invalid shell metric: "..key)
        result[key]=value
    end
    if input.islands~=nil then
        assert(type(input.islands)=="table","Invalid Islands metrics")
        local limits={height={32,72},gap={0,32},edge_gap={4,48},side_gap={4,48},radius={0,32},rail={32,64},padding={0,32}}
        result.islands={}
        for key,range in pairs(limits) do
            local value=input.islands[key]
            if key=="padding" and value==nil then value=16 end
            assert(type(value)=="number" and value%1==0 and value>=range[1] and value<=range[2],"Invalid Islands metric: "..key)
            result.islands[key]=value
        end
        local joined=input.islands.joined_bar
        assert(joined==nil or type(joined)=="boolean","Invalid connected bar preference")
        result.islands.joined_bar=joined==true
        assert(result.frame==0 and result.bar==result.islands.height and result.radius==result.islands.radius and result.rail==result.islands.rail,"Inconsistent Islands metrics")
    end
    return result
end
function enzui_bar_offset(metrics) return metrics.islands and metrics.islands.edge_gap or 0 end
function enzui_bar_band(metrics) return metrics.bar+(metrics.islands and metrics.islands.edge_gap+8 or 0) end
