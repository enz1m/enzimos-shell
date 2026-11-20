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
hl.bind("ALT + Tab", function() enzui_switcher_step(1,false) end, { repeating = true })
hl.bind("ALT + SHIFT + Tab", function() enzui_switcher_step(-1,false) end, { repeating = true })
hl.bind("ALT + grave", function() enzui_switcher_step(1,true) end, { repeating = true })
hl.bind("ALT + Escape", enzui_switcher_cancel)
hl.bind("ALT + Alt_L", enzui_switcher_commit, { release = true, transparent = true, non_consuming = true })
hl.bind("ALT + Alt_R", enzui_switcher_commit, { release = true, transparent = true, non_consuming = true })
hl.bind("SUPER + period", command("emoji-picker"))
-- Dictation: hold to talk, or tap for hands-free; the daemon tells them apart.
hl.bind("SUPER + H", hl.dsp.exec_cmd("enzimos-dictation press"))
hl.bind("SUPER + H", hl.dsp.exec_cmd("enzimos-dictation release"), { release = true })
hl.bind("SUPER + W", command("window-spread"))
-- Locking goes through the shell, which supervises the lock client.
hl.bind("SUPER + L", command("lock"))
hl.bind("CTRL + SHIFT + ALT + R", command("record"))
hl.bind("Print", command("capture"))
hl.bind("SHIFT + Print", command("screenshot-screen"))
hl.bind("ALT + Print", command("screenshot-window"))
hl.bind("SUPER + ALT + equal", command("zoom-in"), level)
hl.bind("SUPER + ALT + minus", command("zoom-out"), level)
hl.bind("SUPER + ALT + 8", command("zoom-reset"), { locked = true })
