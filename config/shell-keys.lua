-- Shell bindings emit enzui-command events; level keys repeat and remain available while
-- locked.
local function command(name)
    return function() hl.dispatch(hl.dsp.event("enzui-command:" .. name)) end
end
local level = { locked = true, repeating = true }
hl.bind("XF86AudioRaiseVolume", command("volume-up"), level)
hl.bind("XF86AudioLowerVolume", command("volume-down"), level)
hl.bind("XF86AudioMute", command("volume-mute"), { locked = true })
hl.bind("XF86AudioMicMute", command("microphone-mute"), { locked = true })
hl.bind("XF86MonBrightnessUp", command("brightness-up"), level)
hl.bind("XF86MonBrightnessDown", command("brightness-down"), level)
hl.bind("XF86AudioPlay", command("media-play-pause"), { locked = true })
hl.bind("XF86AudioPause", command("media-play-pause"), { locked = true })
hl.bind("XF86AudioNext", command("media-next"), { locked = true })
hl.bind("XF86AudioPrev", command("media-previous"), { locked = true })
hl.bind("XF86AudioStop", command("media-stop"), { locked = true })
-- Focus commits in the key callback before ordinary keys can reach the old surface.
dofile(debug.getinfo(1,"S").source:sub(2):match("^(.*)/") .. "/window-switcher.lua")
local function switch(key, direction, same_app)
    local function modifier()
        local chord = enzui_shortcut_chord(key)
        for _, mod in ipairs({"ALT", "SUPER", "CTRL", "SHIFT"}) do
            if chord:upper():match("%f[%a]" .. mod .. "%f[%A] *%+") then return mod end
        end
    end
    hl.bind(key, function()
        enzui_switcher_step(direction, same_app)
        if enzui_switcher.active then enzui_switcher.hold = modifier() end
    end, { repeating = true })
    hl.bind(key, function()
        if not modifier() and not enzui_switcher.hold then enzui_switcher_commit() end
    end, { release = true, non_consuming = true })
end
switch("ALT + Tab", 1, false)
switch("ALT + SHIFT + Tab", -1, false)
switch("ALT + code:49", 1, true)
hl.bind("ALT + Escape", function()
    if enzui_switcher.active then return enzui_switcher_cancel() end
    return {ok=true, pass_event=true}
end)
for mod, keys in pairs({
    ALT = {"Alt_L", "Alt_R"}, SUPER = {"Super_L", "Super_R"},
    CTRL = {"Control_L", "Control_R"}, SHIFT = {"Shift_L", "Shift_R"},
}) do
    for _, key in ipairs(keys) do
        hl.bind(key, function()
            if enzui_switcher.hold == mod then enzui_switcher_commit() end
        end, { release = true, ignore_mods = true, transparent = true, non_consuming = true })
    end
end
hl.bind("SUPER + SPACE", command("search"))
hl.bind("SUPER + A", command("launcher"))
hl.bind("SUPER + code:61", command("shortcuts"))
hl.bind("SUPER + C", command("controls"))
hl.bind("SUPER + SHIFT + V", command("desktop-layout"))
hl.bind("SUPER + N", command("notifications"))
hl.bind("SUPER + SHIFT + D", command("dock"))
hl.bind("SUPER + D", command("overview"))
hl.bind("SUPER + ALT + K", command("keyboard"))
hl.bind("SUPER + code:60", command("emoji-picker"))
-- Dictation: hold to talk, or tap for hands-free; the daemon tells them apart.
hl.bind("SUPER + H", hl.dsp.exec_cmd("enzimos-dictation press"))
hl.bind("SUPER + H", hl.dsp.exec_cmd("enzimos-dictation release"), { release = true })
hl.bind("SUPER + W", command("window-spread"))
-- Super pressed and released with no other key, button or wheel between opens
-- the overview; Super+key shortcuts and Super+drag do not. A press runs no
-- bind, so the Super key still reaches apps.
enzui_super_tap = enzui_super_tap or { pointer = 0 }
local tap = enzui_super_tap
if hl.on and not tap.listener then
    tap.listener = hl.on("input.keyboard.key", function(code, _, state)
        if state == 1 then
            tap.pressed, tap.pointer_at_press = code, tap.pointer
        else
            tap.released = code
        end
    end)
end
for _, input in ipairs({ "mouse:272", "mouse:273", "mouse:274", "mouse_up", "mouse_down", "mouse_left", "mouse_right" }) do
    hl.bind(input, function() tap.pointer = tap.pointer + 1 end, { non_consuming = true, ignore_mods = true })
end
local function super_released()
    -- The key event precedes its binds: the released key is the one pressed last.
    if tap.released == nil or tap.released ~= tap.pressed or tap.pointer ~= tap.pointer_at_press then return end
    local desktop = hl.get_active_workspace and hl.get_active_workspace()
    local spread = not enzui_desktop_fullscreen() and not enzui_desktop_tiling_at(desktop and desktop.id)
    hl.dispatch(hl.dsp.event("enzui-command:" .. (spread and "activities" or "activities-search")))
end
hl.bind("SUPER + Super_L", super_released, { release = true })
hl.bind("SUPER + Super_R", super_released, { release = true })
-- Locking goes through the shell, which supervises the lock client.
hl.bind("SUPER + L", command("lock"))
hl.bind("CTRL + ALT + SHIFT + R", command("record"))
hl.bind("Print", command("capture"))
hl.bind("SHIFT + Print", command("screenshot-screen"))
hl.bind("ALT + Print", command("screenshot-window"))
hl.bind("SUPER + ALT + code:21", command("zoom-in"), level)
hl.bind("SUPER + ALT + code:20", command("zoom-out"), level)
hl.bind("SUPER + ALT + code:17", command("zoom-reset"), { locked = true })
