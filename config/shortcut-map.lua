-- Remap live and future bindings through saved chord choices without replacing binding handles.
enzui_raw_bind = enzui_raw_bind or hl.bind
enzui_raw_unbind = enzui_raw_unbind or hl.unbind
enzui_shortcut_map = enzui_shortcut_map or {}
enzui_shortcut_records = {}

local function chosen(key)
    return enzui_shortcut_map[key] or key
end

local function record_proxy(record)
    return setmetatable({}, {
        __index = function(_, name)
            local handle = record.handle
            if not handle then return nil end
            local value = handle[name]
            if type(value) ~= "function" then return value end
            return function(_, ...)
                if name == "remove" or name == "unbind" then
                    enzui_shortcut_records[record] = nil
                elseif name == "set_enabled" then
                    record.enabled = ...
                end
                return value(record.handle, ...)
            end
        end,
    })
end

hl.bind = function(key, action, options)
    local handle = enzui_raw_bind(chosen(key), action, options)
    if not handle then return handle end
    local record = { key = key, action = action, options = options, handle = handle, enabled = true }
    enzui_shortcut_records[record] = true
    return record_proxy(record)
end

hl.unbind = function(key)
    for record in pairs(enzui_shortcut_records) do
        if record.key == key then enzui_shortcut_records[record] = nil end
    end
    return enzui_raw_unbind(chosen(key))
end

function enzui_set_shortcuts(map)
    local moved = {}
    for record in pairs(enzui_shortcut_records) do
        if (map[record.key] or record.key) ~= chosen(record.key) then
            record.handle:remove()
            moved[#moved + 1] = record
        end
    end
    enzui_shortcut_map = map
    for _, record in ipairs(moved) do
        record.handle = enzui_raw_bind(chosen(record.key), record.action, record.options)
        if record.handle and record.enabled == false then record.handle:set_enabled(false) end
    end
    return "ok"
end

-- Pause bindings during shortcut capture; timeout releases an abandoned capture submap.
hl.define_submap("enzui-recording", function()
    enzui_raw_bind("SUPER + CTRL + ALT + SHIFT + F24", hl.dsp.submap("reset"))
end)
function enzui_shortcut_recording(on)
    enzui_shortcut_recording_serial = (enzui_shortcut_recording_serial or 0) + 1
    local serial = enzui_shortcut_recording_serial
    if on then
        hl.dispatch(hl.dsp.submap("enzui-recording"))
        hl.timer(function()
            if enzui_shortcut_recording_serial == serial then hl.dispatch(hl.dsp.submap("reset")) end
        end, { timeout = 15000, type = "oneshot" })
    else
        hl.dispatch(hl.dsp.submap("reset"))
    end
    return "ok"
end
