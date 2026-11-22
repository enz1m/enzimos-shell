local function desktop_scale(monitor)
    local width, height = monitor.width, monitor.height
    local mm_w, mm_h = monitor.physical_width, monitor.physical_height
    if not mm_w or not mm_h or mm_w < 50 or mm_h < 50 then return 1 end
    local ppi = math.sqrt(width^2 + height^2) * 25.4 / math.sqrt(mm_w^2 + mm_h^2)
    if ppi < 140 or ppi > 400 then return 1 end

    -- Keep a usable work area; the compositor rounds fractional logical sizes.
    local preferred = ppi >= 190 and 2 or ppi >= 160 and 1.5 or 1.25
    for _, scale in ipairs({2, 1.5, 1.25}) do
        local w, h = width / scale, height / scale
        if scale <= preferred and math.max(w, h) >= 1024 and math.min(w, h) >= 640 then
            return scale
        end
    end
    return 1
end

hl.on("hyprland.start", function()
    for _, monitor in ipairs(hl.get_monitors()) do
        if not monitor.name:match("^ENZREMOTE%-") then
            local scale = desktop_scale(monitor)
            if scale ~= monitor.scale then
                hl.monitor({output = monitor.name, scale = scale})
            end
        end
    end
end)
