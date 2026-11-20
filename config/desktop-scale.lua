-- Unknown physical dimensions must never turn a small panel into a 2x desktop.
local function desktop_scale(width, height)
    for _, scale in ipairs({2, 1.6, 1.5, 1.25}) do
        local w, h = width / scale, height / scale
        if w >= 1280 and h >= 800 and w == math.floor(w) and h == math.floor(h) then return scale end
    end
    return 1
end
hl.on("hyprland.start", function()
    for _, monitor in ipairs(hl.get_monitors()) do
        if not monitor.name:match("^ENZREMOTE%-") then
            hl.monitor({output=monitor.name, scale=desktop_scale(monitor.width, monitor.height)})
        end
    end
end)
