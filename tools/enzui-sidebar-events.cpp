#include "src/plugins/PluginAPI.hpp"
#undef PLUGIN_INIT
#undef PLUGIN_EXIT
#define PLUGIN_INIT pointer_plugin_init
#define PLUGIN_EXIT pointer_plugin_exit
#include "enzui-pointer-events.cpp"
#undef PLUGIN_INIT
#undef PLUGIN_EXIT
#include "../compositor/SidebarScene.cpp"
#include "../compositor/Material.cpp"
#include "../compositor/NestedKeyboard.cpp"

APICALL EXPORT PLUGIN_DESCRIPTION_INFO pluginInit(HANDLE handle) {
    auto description = pointer_plugin_init(handle);
    SidebarScene::init(handle);
    EnzMaterial::init(handle);
    NestedKeyboard::init(handle);
    return description;
}
APICALL EXPORT void pluginExit() {
    NestedKeyboard::exit();
    EnzMaterial::exit();
    SidebarScene::exit(owner);
    pointer_plugin_exit();
}
