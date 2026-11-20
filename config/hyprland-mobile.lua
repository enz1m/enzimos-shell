-- Standalone portrait preset; ENZUI_MOBILE_MODE and ENZUI_MOBILE_SCALE override geometry.

local mode = os.getenv("ENZUI_MOBILE_MODE") or "544x1216"
local scale = tonumber(os.getenv("ENZUI_MOBILE_SCALE") or "1") or 1

hl.monitor({
    output = "",
    mode = mode,
    position = "0x0",
    scale = scale,
})

hl.env("XCURSOR_SIZE", "24")
hl.env("HYPRCURSOR_SIZE", "24")

hl.config({
    general = {
        gaps_in = 0,
        gaps_out = 0,
        float_gaps = 0,
        border_size = 0,
        resize_on_border = false,
        col = {
            active_border = "rgba(00000000)",
            inactive_border = "rgba(00000000)",
        },
    },
    decoration = {
        rounding = 0,
        shadow = {
            enabled = false,
        },
    },
    animations = {
        enabled = false,
    },
    input = {
        follow_mouse = 1,
    },
    misc = {
        disable_hyprland_logo = true,
        disable_splash_rendering = true,
        force_default_wallpaper = 0,
    },
    ecosystem = {
        no_update_news = true,
        no_donation_nag = true,
    },
    dwindle = {
        force_split = 2,
    },
})

-- Maximize respects client minimum sizes; fullscreen does not provide the same layout.
hl.window_rule({
    name = "mobile-maximize",
    match = { class = ".*" },
    maximize = true,
    border_size = 0,
    rounding = 0,
    decorate = false,
    no_shadow = true,
})

hl.bind("SUPER + Return", hl.dsp.exec_cmd('exec "${ENZUI_TERMINAL:-foot}"'))
hl.bind("SUPER + Q", hl.dsp.window.close())
