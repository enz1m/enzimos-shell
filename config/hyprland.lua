local mod = "SUPER"
local directory = debug.getinfo(1, "S").source:sub(2):match("^(.*)/")
dofile(directory .. "/shortcut-map.lua")
dofile(directory .. "/shell-metrics.lua")
dofile(directory .. "/desktop-policy.lua")
dofile(directory .. "/panel-policy.lua")
dofile(directory .. "/display-frame.lua")
dofile(directory .. "/desktop-scale.lua")
dofile(directory .. "/wallpaper-layer.lua")

hl.monitor({ output = "", mode = "1440x900@60", position = "0x0", scale = 1, reserved = { top = 36 } })
hl.env("XCURSOR_SIZE", "24")
hl.env("HYPRCURSOR_SIZE", "24")
hl.env("ENZUI_LOCK_XRAY", "1")
hl.config({
    general = {
        gaps_in = 6, gaps_out = 12, float_gaps = 12, border_size = 2, resize_on_border = true,
        -- The focused window's frame and shadow are a step stronger than the
        -- others', so the active window (and the focused tile) can be found.
        col = { active_border = "rgba(92969da8)", inactive_border = "rgba(55585d30)" },
    },
    decoration = {
        rounding = 12,
        blur = { enabled = false },
        shadow = { enabled = true, range = 18, render_power = 3, color = "rgba(00000034)", color_inactive = "rgba(00000016)" },
        -- Applications that leave their frame to the compositor get a title bar; the Shell
        -- gives it the enzUI theme.
        titlebar = { enabled = true },
        -- Quick Search and the modal dialogs darken the rest of the screen slightly.
        dim_around = 0.22,
    },
    animations = { enabled = true },
    binds = { drag_center_window = false },
    input = { follow_mouse = 2, float_switch_override_focus = 0 },
    misc = {
        -- Allow a replacement client to recover the prompt while compositor enforcement remains
        -- active.
        allow_session_lock_restore = true,
        -- Keep desktop rendering beneath lock fades; client failure still leaves outputs
        -- covered.
        session_lock_xray = true,
        -- The panel belongs inside the reserved band, not the application work area.
        new_float_force_onscreen = 0,
        disable_hyprland_logo = true, disable_splash_rendering = true, force_default_wallpaper = 0,
        -- The desktop supervisor and portal environment own these integration checks.
        disable_watchdog_warning = true, disable_hyprland_guiutils_check = true, disable_xdg_env_checks = true,
    },
    ecosystem = { no_update_news = true, no_donation_nag = true },
    -- Use the requested scale and round the logical size in Shell layout.
    debug = { disable_scale_checks = true },
})
hl.curve("desktop-ease", { type = "bezier", points = { { 0.2, 0.8 }, { 0.2, 1.0 } } })
hl.animation({ leaf = "global", enabled = true, speed = 3, bezier = "desktop-ease" })
hl.animation({ leaf = "workspaces", enabled = true, speed = 3, bezier = "desktop-ease", style = "slide" })

hl.window_rule({
    name = "desktop-panel", match = { class = "^dev[.]enzui[.]Desktop$" },
    float = true, pin = true, move = "0 0", size = "monitor_w 36",
    min_size = "monitor_w 36", max_size = "monitor_w 36",
    border_size = 0, rounding = 0, decorate = false, no_shadow = true, no_anim = true, no_initial_focus = true,
})
hl.window_rule({
    name = "desktop-launcher", match = { class = "^dev[.]enzui[.]Launcher$" },
    float = true, center = true, size = "720 500", border_size = 0, rounding = 16,
    decorate = false, no_anim = true,
})

hl.window_rule({
    name = "desktop-quick-search", match = { class = "^dev[.]enzui[.]QuickSearch$" },
    float = true, move = "10000 10000", no_initial_focus = true, border_size = 0, rounding = 0,
    decorate = false, no_anim = true, no_shadow = true, no_xdg_drags = true,
    suppress_event = "maximize fullscreen",
})

hl.window_rule({
    name = "desktop-authentication", match = { class = "^dev[.]enzui[.]Authentication$" },
    float = true, center = true, pin = true, size = "440 330", border_size = 0, rounding = 16,
    decorate = false, stay_focused = true, dim_around = true,
})
hl.window_rule({
    name = "desktop-keyring", match = { class = "^dev[.]enzui[.]KeyringPrompt$" },
    float = true, center = true, pin = true, border_size = 0, rounding = 16,
    decorate = false, stay_focused = true, dim_around = true,
})
hl.window_rule({
    name = "desktop-network-secrets", match = { class = "^dev[.]enzui[.]NetworkSecrets$" },
    float = true, center = true, pin = true, border_size = 0, rounding = 16,
    decorate = false, stay_focused = true, dim_around = true,
})
hl.window_rule({
    name = "desktop-file-chooser", match = { class = "^dev[.]enzui[.]FileChooser$" },
    float = true, center = true, size = "min(1000,monitor_w-48) min(660,monitor_h-96)",
})
hl.window_rule({
    name = "desktop-portal-access", match = { class = "^dev[.]enzui[.]PortalAccess$" },
    float = true, center = true, pin = true, size = "460 280", border_size = 0, rounding = 16,
    decorate = false, stay_focused = true, dim_around = true,
})
hl.window_rule({
    name = "desktop-share-picker", match = { class = "^dev[.]enzui[.]SharePicker$" },
    float = true, center = true, pin = true, border_size = 0, rounding = 16,
    decorate = false, dim_around = true,
})
hl.window_rule({
    -- The dialog sizes itself: taller when it shows a call's arguments.
    name = "desktop-agent-consent", match = { class = "^dev[.]enzui[.]AgentConsent$" },
    float = true, center = true, pin = true, border_size = 0, rounding = 16,
    decorate = false, stay_focused = true, dim_around = true,
})

-- Report presses without consuming them so Search can detect outside clicks.
hl.bind("mouse:272", hl.dsp.event("enzui-command:pointer-press"), { non_consuming = true, ignore_mods = true })
local open_terminal = hl.dsp.exec_cmd('exec "${ENZUI_TERMINAL:-foot}"')
hl.bind(mod .. " + Return", open_terminal)
enzui_bind_terminal_alias(open_terminal)
-- Super+Alt+S starts or stops the Orca screen reader, also while locked.
hl.bind(mod .. " + ALT + S", hl.dsp.exec_cmd('pkill -u "$(id -u)" -x orca || exec orca --replace'), { locked = true })
for i = 1, 9 do
    hl.bind(mod .. " + " .. i, hl.dsp.focus({ workspace = i }))
end
dofile(directory .. "/panel-policy.lua")
dofile(directory .. "/shell-keys.lua")
