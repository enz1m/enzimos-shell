-- Private greeter compositor; exit returns session launch or recovery to greetd.
hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })
hl.env("XCURSOR_SIZE", "24")
hl.env("HYPRCURSOR_SIZE", "24")
hl.config({
    animations = { enabled = false },
    misc = {
        -- A crashed greeter can be restarted over its still-locked outputs.
        allow_session_lock_restore = true,
        disable_hyprland_logo = true, disable_splash_rendering = true, force_default_wallpaper = 0,
        -- The desktop supervises its compositor itself and ships no Hyprland dialogs.
        disable_watchdog_warning = true, disable_hyprland_guiutils_check = true, disable_xdg_env_checks = true,
        background_color = 0xff000000,
    },
    ecosystem = { no_update_news = true, no_donation_nag = true },
    -- Disable Xwayland to avoid aborting an unused X server at sign-in.
    xwayland = { enabled = false },
})
-- Read the system keyboard configuration before accepting passwords.
do
    local file=io.open("/etc/X11/xorg.conf.d/00-keyboard.conf","r")
    if file then
        local text=file:read(65536) or ""
        file:close()
        local function option(name) return text:match('Option%s+"'..name..'"%s+"([^"]*)"') end
        local input={}
        local layout,variant,options=option("XkbLayout"),option("XkbVariant"),option("XkbOptions")
        if layout and layout:match("^[a-z0-9_,]+$") then input.kb_layout=layout end
        if variant and variant:match("^[a-z0-9_,-]*$") and variant~="" then input.kb_variant=variant end
        if options and options:match("^[a-z0-9:_,-]*$") and options~="" then input.kb_options=options end
        if next(input) then hl.config({ input=input }) end
    end
end
-- Super+Alt+S starts or stops the Orca screen reader, before anyone signs in.
hl.bind("SUPER + ALT + S", hl.dsp.exec_cmd('pkill -u "$(id -u)" -x orca || exec orca --replace'), { locked = true })
local function kiosk_scale(width, height)
    for _, scale in ipairs({ 2, 1.6, 1.5, 1.25 }) do
        local w, h = width / scale, height / scale
        if w >= 1280 and h >= 800 and w == math.floor(w) and h == math.floor(h) then
            return scale
        end
    end
    return 1
end
hl.on("hyprland.start", function()
    for _, monitor in ipairs(hl.get_monitors()) do
        hl.monitor({ output = monitor.name, scale = kiosk_scale(monitor.width, monitor.height) })
    end
    hl.exec_cmd("enzui-greeter; \"$(command -v enz-hyprctl || command -v hyprctl)\" dispatch 'hl.dsp.exit()'")
end)
