static bool barFullscreenSuppressed = false;

struct BarRegions {
    PHLWINDOWREF window;
    WP<CWLSurfaceResource> surface;
    Vector2D size;
    CRegion original, wanted;
    uint64_t epoch = 0, serial = 0;
    size_t count = 0;
    bool enabled = false, originalInfinite = true, pending = false;
};
static BarRegions barRegions;
static bool islandsSupported() {
    using Capability = bool (*)();
    static const auto capability = reinterpret_cast<Capability>(dlsym(RTLD_DEFAULT, "enzui_shell_input_regions_v1"));
    return capability && capability();
}
static void restoreBarRegions() {
    if (barRegions.window && barRegions.window->mapped() && barRegions.surface &&
        barRegions.window->backend().surface() == barRegions.surface.lock()) {
        auto& state = barRegions.surface->m_current;
        state.input = barRegions.original;
        state.inputIsInfinite = barRegions.originalInfinite;
    }
    barRegions.enabled = false;
    barRegions.pending = false;
    barRegions.wanted.clear();
    barRegions.count = 0;
    barRegions.window.reset();
    barRegions.surface.reset();
}
static void resetBarRegions() {
    if (bar) bar->setInputBlocked(inputReason, false);
    barFullscreenSuppressed = false;
    restoreBarRegions();
    barRegions = {};
}
static void updateBarRegions() {
    const bool suppressed = fullscreenOnOutput() && Desktop::focusState()->window() != bar;
    if (suppressed != barFullscreenSuppressed) {
        barFullscreenSuppressed = suppressed;
        if (bar) bar->setInputBlocked(inputReason, suppressed);
        g_pInputManager->simulateMouseMovement();
        damage();
    }
    if (!barRegions.enabled)
        return;
    if (!ownerAlive() || barRegions.window != bar || !barRegions.surface ||
        bar->backend().surface() != barRegions.surface.lock()) {
        restoreBarRegions();
        return;
    }
    auto& state = barRegions.surface->m_current;
    barRegions.pending = !state.texture || state.size != barRegions.size || bar->backend().geometry().box.size() != barRegions.size;
    const CRegion empty;
    const auto& wanted = barRegions.pending ? empty : barRegions.wanted;
    if (state.inputIsInfinite || !pixman_region32_equal(state.input.pixman(), wanted.pixman())) {
        state.input = wanted;
        state.inputIsInfinite = false;
        g_pInputManager->simulateMouseMovement();
    }
}
static int islandsSupportedApi(lua_State* L) { return result(L, islandsSupported()); }
static int dockSupportedApi(lua_State* L) {
    using Capability = bool (*)();
    static const auto capability = reinterpret_cast<Capability>(dlsym(RTLD_DEFAULT, "enzui_dock_input_regions_v1"));
    return result(L, capability && capability());
}
static int barMode(lua_State* L) {
    if (lua_gettop(L) != 5 || !motionOwner(L) || !lua_isinteger(L, 2) ||
        lua_tointeger(L, 2) < 1 || lua_tointeger(L, 2) > 9007199254740991LL ||
        !lua_isboolean(L, 3) || !lua_isnumber(L, 4) || !lua_isnumber(L, 5))
        return result(L, false);
    const auto epoch = static_cast<uint64_t>(lua_tointeger(L, 2));
    const bool enabled = lua_toboolean(L, 3);
    const Vector2D size{lua_tonumber(L, 4), lua_tonumber(L, 5)};
    if (!std::isfinite(size.x) || !std::isfinite(size.y) || size.x < 320 || size.x > 16384 ||
        size.y < 1 || size.y > 128 || (enabled && !islandsSupported()) || epoch < barRegions.epoch)
        return result(L, false);
    if (epoch == barRegions.epoch)
        return result(L, enabled == barRegions.enabled && size == barRegions.size);
    restoreBarRegions();
    barRegions.epoch = epoch;
    barRegions.serial = 0;
    barRegions.size = size;
    if (!enabled)
        return result(L, true);
    const auto surface = bar->backend().surface();
    if (!surface)
        return result(L, false);
    barRegions.window = bar;
    barRegions.surface = surface;
    barRegions.original = surface->m_current.input;
    barRegions.originalInfinite = surface->m_current.inputIsInfinite;
    barRegions.enabled = true;
    updateBarRegions();
    damage();
    return result(L, true);
}
static int setBarRegions(lua_State* L) {
    if (lua_gettop(L) != 4 || !motionOwner(L) || !barRegions.enabled ||
        !lua_isinteger(L, 2) || !lua_isinteger(L, 3) || !lua_istable(L, 4) ||
        lua_tointeger(L, 2) != static_cast<int64_t>(barRegions.epoch) ||
        lua_tointeger(L, 3) < 1 || lua_tointeger(L, 3) > 9007199254740991LL ||
        static_cast<uint64_t>(lua_tointeger(L, 3)) < barRegions.serial || lua_rawlen(L, 4) > 64)
        return result(L, false);
    CRegion wanted;
    const auto count = lua_rawlen(L, 4);
    for (size_t i = 1; i <= count; ++i) {
        lua_rawgeti(L, 4, i);
        if (!lua_istable(L, -1) || lua_rawlen(L, -1) != 8) {
            lua_pop(L, 1);
            return result(L, false);
        }
        double values[8];
        for (int j = 0; j < 8; ++j) {
            lua_rawgeti(L, -1, j + 1);
            const bool valid = lua_isnumber(L, -1) && std::isfinite(lua_tonumber(L, -1));
            values[j] = valid ? lua_tonumber(L, -1) : 0;
            lua_pop(L, 1);
            if (!valid) {
                lua_pop(L, 1);
                return result(L, false);
            }
        }
        lua_pop(L, 1);
        const CBox box{values[0], values[1], values[2], values[3]};
        const CBox clip{values[4], values[5], values[6], values[7]};
        if (std::abs(box.x) > 32768 || std::abs(box.y) > 128 || box.w <= 0 || box.w > 16384 ||
            box.h <= 0 || box.h > 128 || clip.x < 0 || clip.y < 0 || clip.w <= 0 || clip.h <= 0 ||
            clip.x + clip.w > barRegions.size.x + .01 || clip.y + clip.h > barRegions.size.y + .01)
            return result(L, false);
        const double radius = std::min(box.w, box.h) / 2;
        CRegion capsule;
        for (int y = std::max(0, static_cast<int>(std::floor(clip.y))); y < std::ceil(clip.y + clip.h); ++y) {
            const double row = y + .5;
            if (row < box.y || row >= box.y + box.h)
                continue;
            const double distance = std::max({box.y + radius - row, row - (box.y + box.h - radius), 0.0});
            const double inset = radius - std::sqrt(std::max(0.0, radius * radius - distance * distance));
            const double left = std::max(box.x + inset, clip.x);
            const double end = std::min(box.x + box.w - inset, clip.x + clip.w);
            const int x = static_cast<int>(std::ceil(left));
            const int width = static_cast<int>(std::floor(end)) - x;
            if (width > 0)
                capsule.add(x, y, width, 1);
        }
        wanted.add(capsule);
    }
    if (static_cast<uint64_t>(lua_tointeger(L, 3)) == barRegions.serial)
        return result(L, count == barRegions.count && pixman_region32_equal(wanted.pixman(), barRegions.wanted.pixman()));
    barRegions.wanted = std::move(wanted);
    barRegions.serial = lua_tointeger(L, 3);
    barRegions.count = count;
    updateBarRegions();
    damage();
    return result(L, true);
}
static int barRegionsState(lua_State* L) {
    if (lua_gettop(L) != 1 || !motionOwner(L)) { lua_pushnil(L); return 1; }
    lua_newtable(L);
    lua_pushboolean(L, barRegions.enabled); lua_setfield(L, -2, "enabled");
    lua_pushboolean(L, barFullscreenSuppressed); lua_setfield(L, -2, "suppressed");
    lua_pushboolean(L, barRegions.pending); lua_setfield(L, -2, "pending");
    lua_pushinteger(L, barRegions.epoch); lua_setfield(L, -2, "epoch");
    lua_pushinteger(L, barRegions.serial); lua_setfield(L, -2, "serial");
    lua_pushinteger(L, barRegions.count); lua_setfield(L, -2, "count");
    return 1;
}
