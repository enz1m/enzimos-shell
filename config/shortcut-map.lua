enzui_raw_bind = enzui_raw_bind or hl.bind
enzui_raw_unbind = enzui_raw_unbind or hl.unbind
local choices, records, custom = {}, {}, {}
local custom_handles = {}

function enzui_shortcut_chord(key)
    return choices[key] or key
end

local function remove(record)
    if record.handle then record.handle:remove(); record.handle = nil end
end

local function install(record)
    local chord = enzui_shortcut_chord(record.key)
    if chord == "" then return end
    record.handle = assert(enzui_raw_bind(chord, record.action, record.options))
    if not record.enabled then record.handle:set_enabled(false) end
end

hl.bind = function(key, action, options)
    local record = { key = key, action = action, options = options, enabled = true }
    install(record)
    records[record] = true
    return {
        remove = function()
            records[record] = nil
            remove(record)
        end,
        unbind = function()
            records[record] = nil
            remove(record)
        end,
        set_enabled = function(_, enabled)
            record.enabled = enabled
            if record.handle then return record.handle:set_enabled(enabled) end
        end,
    }
end

hl.unbind = function(key)
    local found = false
    for record in pairs(records) do
        if record.key == key then
            records[record] = nil
            remove(record)
            found = true
        end
    end
    if not found and choices[key] == nil then return enzui_raw_unbind(key) end
end

local function install_custom(rows)
    for _, row in ipairs(rows) do
        local action = row.command and hl.dsp.exec_cmd(row.command) or
            function() hl.dispatch(hl.dsp.event("enzui-command:" .. row.event)) end
        custom_handles[row.id] = assert(enzui_raw_bind(row.chord, action))
    end
end

local function remove_custom()
    for _, handle in pairs(custom_handles) do handle:remove() end
    custom_handles = {}
end

function enzui_set_shortcuts(map, rows)
    local moved = {}
    for record in pairs(records) do
        if (map[record.key] or record.key) ~= enzui_shortcut_chord(record.key) then
            remove(record)
            moved[#moved + 1] = record
        end
    end
    remove_custom()
    local previous = choices
    choices = map
    local ok, reason = pcall(function()
        for _, record in ipairs(moved) do install(record) end
        install_custom(rows)
    end)
    if not ok then
        for _, record in ipairs(moved) do remove(record) end
        remove_custom()
        choices = previous
        for _, record in ipairs(moved) do install(record) end
        install_custom(custom)
        error(reason)
    end
    custom = rows
    return "ok"
end

function enzui_shortcut_snapshot(defaults)
    local active = {}
    for record in pairs(records) do
        if record.enabled and record.handle then active[record.key] = true end
    end
    local rows = {}
    for _, key in ipairs(defaults) do
        if active[key] then rows[#rows + 1] = key .. "\t" .. enzui_shortcut_chord(key) end
    end
    for _, row in ipairs(custom) do rows[#rows + 1] = row.id .. "\t" .. row.chord end
    return table.concat(rows, "\n")
end

-- An abandoned capture must release the compositor's keyboard submap.
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
