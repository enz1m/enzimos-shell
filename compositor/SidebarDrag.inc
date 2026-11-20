static PHLWINDOW windowAddress(const char* value) {
    if (!value) return nullptr;
    for (const auto& w : Desktop::windowState()->windows())
        if (w->mapped() && address(w) == value) return w;
    return nullptr;
}
struct PinDrag {
    PHLWINDOWREF window;
    PHLMONITORREF monitor;
    pid_t owner = 0;
    Vector2D point;
    Time::steady_tp released;
    bool dropped = false;
};
static PinDrag pinDrag;
static CHyprSignalListener pinMotionListener, pinEndedListener;
static void samplePinDrag() {
    const auto& drag = g_layoutManager->dragController();
    const auto target = drag->target();
    const auto w = target ? target->window() : nullptr;
    const auto point = g_pInputManager->getMouseCoordsInternal();
    static auto threshold = CConfigValue<Config::INTEGER>("binds:drag_threshold");
    if (!ownerAlive() || !iconDrop.held || drag->mode() != MBIND_MOVE || !supported(w) || w->m_monitor != output ||
        std::hypot(point.x - iconDrop.pressX, point.y - iconDrop.pressY) <= std::max(8.0, double(*threshold))) return;
    pinDrag = {w, output, ownerPid, point, {}, false};
}
static int pinDragState(lua_State* L) {
    if (lua_gettop(L) != 1 || !motionOwner(L)) { lua_pushnil(L); return 1; }
    const auto& drag = g_layoutManager->dragController();
    const auto target = drag->target();
    const auto w = target ? target->window() : nullptr;
    lua_pushstring(L, std::format("held={} mode={} threshold={} supported={} output={} saved={} owner={}",
        iconDrop.held, int(drag->mode()), drag->dragThresholdReached(), supported(w), w && w->m_monitor == output,
        bool(pinDrag.window), ownerAlive()).c_str());
    return 1;
}
static int pinDragActive(lua_State* L) {
    if (lua_gettop(L) != 2 || !motionOwner(L) || !lua_isstring(L, 2)) return result(L, false);
    samplePinDrag();
    const auto target = g_layoutManager->dragController()->target();
    return result(L, iconDrop.held && pinDrag.owner == ownerPid && pinDrag.monitor == output && pinDrag.window &&
        target && target->window() == pinDrag.window && address(pinDrag.window.lock()) == lua_tostring(L, 2));
}
static int pinDragDrop(lua_State* L) {
    if (lua_gettop(L) != 2 || !motionOwner(L) || !lua_isstring(L, 2)) return result(L, false);
    const auto now = Time::steadyNow();
    const bool valid = !iconDrop.held && pinDrag.dropped && pinDrag.owner == ownerPid && pinDrag.monitor == output && pinDrag.window &&
        pinDrag.window->mapped() && address(pinDrag.window.lock()) == lua_tostring(L, 2) &&
        now >= pinDrag.released && now - pinDrag.released < std::chrono::seconds(1) &&
        (g_pInputManager->getMouseCoordsInternal() - pinDrag.point).size() <= 2;
    if (valid) pinDrag = {};
    return result(L, valid);
}
