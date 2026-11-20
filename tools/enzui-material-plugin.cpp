#include "src/plugins/PluginAPI.hpp"
#include "../compositor/Material.cpp"

APICALL EXPORT std::string PLUGIN_API_VERSION() {
    return HYPRLAND_API_VERSION;
}

APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    if (std::string_view(__hyprland_api_get_hash()) != __hyprland_api_get_client_hash())
        throw std::runtime_error("enzUI material plugin does not match this compositor ABI");
    EnzMaterial::init(handle);
    return {"enzui-material", "enzUI liquid glass behind client surfaces", "enzUI", "1"};
}

APICALL EXPORT void PLUGIN_EXIT() {
    EnzMaterial::exit();
}
