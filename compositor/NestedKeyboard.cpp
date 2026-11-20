// Recovers modifiers lost when a nested compositor loses parent keyboard focus.
#include "src/plugins/PluginAPI.hpp"
#include "src/devices/Keyboard.hpp"
#include "src/managers/input/InputManager.hpp"
#include <aquamarine/input/Input.hpp>
#include <lua.hpp>
#include <array>
#include <chrono>
#include <cstdlib>
#include <dlfcn.h>
#include <memory>
#include <stdexcept>

namespace NestedKeyboard {
static decltype(g_pInputManager)* inputSlot = nullptr;
struct Keyboard {
    WP<CKeyboard> core;
    CHyprSignalListener modifiers;
};
static std::vector<std::unique_ptr<Keyboard>> keyboards;
static CHyprSignalListener keyListener;
static HANDLE owner = nullptr;
static unsigned long releases = 0;
static constexpr std::array<uint32_t, 8> modifiers = {29, 42, 54, 56, 97, 100, 125, 126};

static void release(const SP<CKeyboard>& core, uint32_t key) {
    auto aq = core->aq();
    if (!aq || !core->getPressed(key))
        return;
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch()).count();
    aq->events.key.emit(Aquamarine::IKeyboard::SKeyEvent{.timeMs = static_cast<uint32_t>(ms), .key = key, .pressed = false});
    ++releases;
}

static void attach() {
    std::erase_if(keyboards, [](const auto& entry) { return entry->core.expired(); });
    for (const auto& input : (*inputSlot)->m_keyboards) {
        auto core = dynamicPointerCast<CKeyboard>(input);
        if (!core || !core->aq() || core->aq()->getName() != "wl_keyboard")
            continue;
        if (std::ranges::any_of(keyboards, [&](const auto& entry) { return entry->core.lock() == core; }))
            continue;
        auto entry = std::make_unique<Keyboard>();
        entry->core = core;
        entry->modifiers = core->aq()->events.modifiers.listen([weak = WP<CKeyboard>(core)](const Aquamarine::IKeyboard::SModifiersEvent& event) {
            // Parent XKB masks may use different indices. A zero depressed/latched
            // mask is unambiguous; locked Caps/Num state must not block recovery.
            if (event.depressed || event.latched)
                return;
            if (auto core = weak.lock()) {
                for (auto key : modifiers)
                    release(core, key);
            }
        });
        keyboards.push_back(std::move(entry));
    }
}

static int status(lua_State* state) {
    attach();
    unsigned heldSuper = 0;
    for (const auto& entry : keyboards) {
        if (auto core = entry->core.lock()) {
            heldSuper |= core->getPressed(125) ? 1 : 0;
            heldSuper |= core->getPressed(126) ? 2 : 0;
        }
    }
    lua_pushinteger(state, keyboards.size());
    lua_pushinteger(state, releases);
    lua_pushinteger(state, static_cast<unsigned>((*inputSlot)->getModsFromAllKBs()));
    lua_pushinteger(state, heldSuper);
    return 4;
}
static int recoverSuper(lua_State* state) {
    attach();
    auto before = releases;
    std::vector<SP<CKeyboard>> targets;
    for (const auto& entry : keyboards) {
        if (auto core = entry->core.lock())
            targets.push_back(core);
    }
    for (const auto& core : targets) {
        release(core, 125);
        release(core, 126);
    }
    lua_pushinteger(state, releases - before);
    return 1;
}

// Nested-only modifier recovery: Aquamarine does not handle the parent keyboard's leave event.
inline bool init(HANDLE handle) {
    const auto parent = std::getenv("ENZUI_HYPR_HOST_WAYLAND");
    if (!parent || !*parent)
        return false;
    inputSlot = static_cast<decltype(g_pInputManager)*>(dlsym(RTLD_DEFAULT, "g_pInputManager"));
    if (!inputSlot || !*inputSlot)
        throw std::runtime_error("Nested keyboard observer cannot locate the input manager");
    owner = handle;
    if (!HyprlandAPI::addLuaFunction(owner, "enzui_nested_keyboard", "status", status)
        || !HyprlandAPI::addLuaFunction(owner, "enzui_nested_keyboard", "recover_super", recoverSuper))
        throw std::runtime_error("Cannot register nested keyboard observer");
    attach();
    keyListener = Event::bus()->m_events.input.keyboard.key.listen([](const IKeyboard::SKeyEvent&, const Event::SCallbackInfo&) { attach(); });
    return true;
}

inline void exit() {
    if (!owner)
        return;
    keyListener.reset();
    keyboards.clear();
    HyprlandAPI::removeLuaFunction(owner, "enzui_nested_keyboard", "status");
    HyprlandAPI::removeLuaFunction(owner, "enzui_nested_keyboard", "recover_super");
    owner = nullptr;
}

} // namespace NestedKeyboard
