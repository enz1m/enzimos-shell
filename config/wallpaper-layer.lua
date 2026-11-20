-- Wallpaper opacity is renderer-owned; compositor fades could reveal the retired layer.
hl.layer_rule({
    name = "desktop-wallpaper", match = { namespace = "^wallpaper$" },
    no_anim = true,
})
hl.layer_rule({
    name = "desktop-wallpaper-renderer", match = { namespace = "^enzui-wallpaper$" },
    no_anim = true,
})
