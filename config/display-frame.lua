hl.config({ general = { extend_border_grab_area = 0 } })
hl.layer_rule({
    name = "desktop-frame", match = { namespace = "^enzui-display-frame$" },
    no_anim = true, blur = false,
})
