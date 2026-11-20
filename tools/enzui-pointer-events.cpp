#include "src/plugins/PluginAPI.hpp"
#include <lua.hpp>
#include <stdexcept>

static HANDLE owner = nullptr;
static SP<Event::CEventBus::CCustomEvent> pointerEvent;
static CHyprSignalListener pointerListener;
static CHyprSignalListener reloadListener;
static bool ready = false;
static unsigned long seen = 0;
static unsigned long failed = 0;

static int counters(lua_State* state) { lua_pushinteger(state, seen); lua_pushinteger(state, failed); return 2; }

static int version(lua_State* state) {
    lua_pushinteger(state, ready ? 1 : 0);
    return 1;
}

APICALL EXPORT std::string PLUGIN_API_VERSION() {
    return HYPRLAND_API_VERSION;
}

APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    if (std::string_view(__hyprland_api_get_hash()) != __hyprland_api_get_client_hash())
        throw std::runtime_error("enzUI pointer event plugin does not match this compositor ABI");
    owner = handle;
    using CustomEvent = Event::CEventBus::CCustomEvent;
    pointerEvent = makeShared<CustomEvent>("enzui.pointer_button", std::vector<CustomEvent::eType>{CustomEvent::TYPE_INT, CustomEvent::TYPE_INT});
    if (!HyprlandAPI::addEvent(owner, pointerEvent) || !HyprlandAPI::addLuaFunction(owner, "enzui_pointer_events", "version", version))
        throw std::runtime_error("Cannot register enzUI pointer event observer");
    HyprlandAPI::addLuaFunction(owner, "enzui_pointer_events", "counters", counters);
    reloadListener = Event::bus()->m_events.config.reloaded.listen([] {
        // Lua listeners are recreated on reload while plugin-owned events survive.
        Event::bus()->m_events.pluginEventAdded.emit(pointerEvent);
        ready = true;
    });
    pointerListener = Event::bus()->m_events.input.mouse.button.listen([](const IPointer::SButtonEvent& event, const Event::SCallbackInfo&) {
        ++seen;
        if (event.button == 272 || event.button == 273) {
            auto result = pointerEvent->emit({static_cast<int>(event.button), static_cast<int>(event.state)});
            if (!result) ++failed;
        }
    });
    return {"enzui-pointer-events", "Passive pointer-button events for the enzUI shell", "enzUI", "1"};
}

APICALL EXPORT void PLUGIN_EXIT() {
    pointerListener.reset();
    reloadListener.reset();
    ready = false;
    HyprlandAPI::removeEvent(owner, "enzui.pointer_button");
    HyprlandAPI::removeLuaFunction(owner, "enzui_pointer_events", "version");
    HyprlandAPI::removeLuaFunction(owner, "enzui_pointer_events", "counters");
    pointerEvent.reset();
    owner = nullptr;
}
