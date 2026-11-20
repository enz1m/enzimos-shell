local lines, timers = {}, {}
local fail_next, opened, pipe_closes = false, 0, 0
local pointer = {x = 350, y = 300}
local window = {
    address = "0x123", class = "foot", mapped = true, hidden = false,
    floating = true, pinned = false, fullscreen = 0,
    at = {x = 250, y = 220}, size = {x = 700, y = 520},
    monitor = {name = "WAYLAND-1", x = 0, y = 0, width = 1440, height = 900, scale = 1, transform = 0,
        reserved = {top = 36, bottom = 0, left = 0, right = 0}},
}
local binding, closed, pressed_at
local active_window = window
local windows = {window}
local old_popen = io.popen
io.popen = function()
    opened = opened + 1
    return {write = function(_, line)
        if fail_next then fail_next = false; return nil, "broken pipe" end
        lines[#lines + 1] = line:gsub("\n$", "")
        return true
    end, flush = function() return true end, close = function() pipe_closes = pipe_closes + 1 end}
end
hl = {
    get_active_window = function() return active_window end,
    get_windows = function() return windows end,
    get_cursor_pos = function() return pointer end,
    get_config = function(name)
        if name == "general:border_size" then return 2 end
        if name == "binds:drag_threshold" then return 0 end
        return false
    end,
    config = function(values) assert(values.binds.drag_threshold == 6) end,
    on = function(event, fn)
        if event == "enzui.pointer_button" then binding = fn
        elseif event == "window.close" then closed = fn
        else error(event) end
        return {remove = function() end}
    end,
    timer = function(fn, opts)
        local t = {fn = fn, kind = opts.type, enabled = true}
        function t:set_enabled(value) self.enabled = value end
        timers[#timers + 1] = t
        return t
    end,
    dsp = {window = {fullscreen = function(args) return args end}},
    dispatch = function(args)
        if args.action == "set" then
            window.fullscreen = 1
            window.at = {x = 12, y = 48}
            window.size = {x = 1416, y = 840}
        else
            window.fullscreen = 0
            window.at = {x = pointer.x - 100, y = pointer.y - 80}
            window.size = {x = 700, y = 520}
        end
        return true
    end,
}
local function step(n)
    for _ = 1, n do
        local snapshot = {table.unpack(timers)}
        for _, timer in ipairs(snapshot) do
            if timer.enabled then
                if timer.kind == "oneshot" then timer.enabled = false end
                timer.fn()
            end
        end
    end
end
local function events(from)
    local found = {}
    for i = from + 1, #lines do found[#found + 1] = lines[i] end
    return table.concat(found, "\n")
end
local function expect(condition, description, from)
    assert(condition, description .. ": " .. events(from or 0))
end
local function press() pressed_at = {x = pointer.x, y = pointer.y}; binding(272, 1) end
local function release() binding(272, 0); pressed_at = nil end
local function move(x, y)
    local previous = pointer
    local floating = window.fullscreen == 0
    pointer = {x = x, y = y}
    if pressed_at and floating then
        window.at = {x = window.at.x + x - previous.x, y = window.at.y + y - previous.y}
    end
    if pressed_at and window.fullscreen == 1
        and math.max(math.abs(x - pressed_at.x), math.abs(y - pressed_at.y)) > 6 then
        hl.dispatch({action = "unset"})
    end
    step(1)
end

dofile("config/output-size.lua")
dofile("config/drag-maximize.lua")
assert(enzui_drag_maximize_install("fake preview"))
press(); move(340, 42)
expect(events(0):find("show ", 1, true), "first drag grows a preview")
release(); step(4)
expect(window.fullscreen == 1, "first drop maximizes")
pointer = {x = 350, y = 55}
local first = #lines
press(); move(370, 130); step(15)
expect(window.fullscreen == 0, "drag restores window")
expect(not events(first):find("show ", 1, true) and not events(first):find("morph ", 1, true),
    "restore never paints an overlay", first)
release()
local second = #lines
fail_next = true
press(); move(380, 42)
expect(events(second):find("show ", 1, true), "second trip to top shows preview", second)
expect(opened == 2, "broken helper pipe is replaced on the same gesture")
release(); step(4)
expect(window.fullscreen == 1, "second drop maximizes")
local active_pipe, active_closes = opened, pipe_closes
assert(enzui_drag_maximize_install("fake preview"))
expect(opened == active_pipe and pipe_closes == active_closes,
    "reinstall retains the preview helper instead of synchronously waiting for it")
assert(enzui_drag_maximize_install("other preview"))
expect(pipe_closes == active_closes and opened == active_pipe and enzui_drag_maximize.pipe == nil,
    "changing output retires the old helper without waiting for its Wayland client")

pointer = {x = 390, y = 55}
local third = #lines
press(); move(410, 160); move(420, 42)
expect(events(third):find("show ", 1, true), "fast return to top re-arms preview", third)
release(); step(4)
expect(window.fullscreen == 1, "fast reverse trip maximizes")

pointer = {x = 420, y = 55}
local fourth = #lines
press(); move(450, 160); move(460, 42); move(460, 180); release(); step(4)
expect(events(fourth):find("show ", 1, true) and events(fourth):find("hide", 1, true),
    "leaving the band cancels its preview", fourth)
expect(window.fullscreen == 0, "leaving the band cancels the drop")

window.fullscreen = 1
window.at, window.size = {x = 12, y = 48}, {x = 1416, y = 840}
pointer = {x = 350, y = 55}
local fifth = #lines
press(); step(2); release()
expect(window.fullscreen == 1 and #lines == fifth, "plain click does not restore or draw")
window.fullscreen = 2
local sixth = #lines
press(); move(600, 42); release(); step(4)
expect(#lines == sixth and window.fullscreen == 2,
    "true fullscreen never participates in a maximize gesture", sixth)
window.fullscreen = 1
window.at, window.size = {x = 12, y = 48}, {x = 1416, y = 840}
pointer = {x = 350, y = 55}
press(); move(500, 180)
closed(window)
expect(enzui_drag_maximize.window == nil and enzui_drag_maximize.timer == nil,
    "closing the dragged window stops its timer")

window.fullscreen = 0
window.at, window.size = {x = 300, y = 35}, {x = 700, y = 520}
pointer = {x = 350, y = 42}
local seventh = #lines
press(); step(2); release(); step(4)
expect(window.fullscreen == 0 and not events(seventh):find("show ", 1, true),
    "clicking a normal window in the top band is not a drag", seventh)
local other = {address="0x987",class="foot",mapped=true,hidden=false,floating=true,pinned=false,fullscreen=0,
    at={x=320,y=205},size={x=200,y=200},monitor=window.monitor}
active_window=other;windows={other,window}
window.at={x=300,y=200};pointer={x=350,y=230}
local eighth=#lines
press();assert(#enzui_drag_maximize.candidates==2 and not enzui_drag_maximize.window)
move(360,44)
assert(enzui_drag_maximize.window==window)
expect(events(eighth):find("show ",1,true),"unfocused dragged window gets the preview",eighth)
release();step(4)
assert(window.fullscreen==1 and other.fullscreen==0,"unfocused drag maximized only its own window")
window.fullscreen=0;window.at={x=300,y=200};pointer={x=350,y=230}
local ninth=#lines
press();pointer={x=360,y=44};step(2);release();step(4)
expect(window.fullscreen==0 and other.fullscreen==0 and not events(ninth):find("show ",1,true),
    "pointer motion alone never maximizes an unmoved window",ninth)
io.popen = old_popen
print("drag-maximize: maximize-only overlay, repeated grow/restore, quick reverse, cancel and click OK")
