static bool nativeTiling = false;
struct TilePlan {
    CBox box;
    PHLWINDOWREF neighbor;
};
static std::optional<TilePlan> tilePlan(PHLWINDOW source, const Vector2D& point) {
    if (!nativeTiling || !ownerAlive() || !source || source->m_monitor != output || remote(source) || !output->m_activeWorkspace ||
        !CBox{output->m_position, output->m_size}.containsPoint(point)) return std::nullopt;
    const auto ws = output->m_activeWorkspace;
    if (Fullscreen::controller()->hasFullscreen(ws)) return std::nullopt;
    PHLWINDOW neighbor;
    CBox box;
    double nearest = 1e100;
    for (const auto& w : Desktop::windowState()->windows()) {
        if (w == source || !supported(w) || w->isFloating() || w->m_workspace != ws || w->m_monitor != output || (w->m_state & Desktop::View::WINDOW_STATE_PINNED)) continue;
        const auto b = w->layoutTarget()->position();
        const double dx = std::max({b.x - point.x, point.x - b.x - b.w, 0.0});
        const double dy = std::max({b.y - point.y, point.y - b.y - b.h, 0.0});
        if (dx * dx + dy * dy < nearest) { nearest = dx * dx + dy * dy; neighbor = w; box = b; }
    }
    const auto space = ws->space();
    if (!space || !space->algorithm()) return std::nullopt;
    if (!neighbor) return TilePlan{space->workArea(), {}};
    if (std::string_view(typeid(*space->algorithm()->tiledAlgo()).name()).find("Dwindle") == std::string_view::npos) return std::nullopt;
    static auto multiplier = CConfigValue<Config::FLOAT>("dwindle:split_width_multiplier");
    static auto ratio = CConfigValue<Config::FLOAT>("dwindle:default_split_ratio");
    static auto smart = CConfigValue<Config::INTEGER>("dwindle:smart_split");
    static auto bias = CConfigValue<Config::INTEGER>("dwindle:split_bias");
    bool horizontal = box.w > box.h * *multiplier;
    if (*smart) horizontal = std::abs((point.x - box.x - box.w / 2) / box.w) > std::abs((point.y - box.y - box.h / 2) / box.h);
    const bool first = horizontal ? point.x < box.x + box.w / 2 : point.y < box.y + box.h / 2;
    const double split = std::clamp(static_cast<double>(*ratio), 0.1, 1.9);
    const double fraction = (first && *bias ? 2.0 - split : split) / 2.0;
    if (horizontal) { if (!first) box.x += box.w * fraction; box.w *= first ? fraction : 1.0 - fraction; }
    else { if (!first) box.y += box.h * fraction; box.h *= first ? fraction : 1.0 - fraction; }
    return TilePlan{box, neighbor};
}
static int tilingMode(lua_State* L) {
    if (lua_gettop(L) != 2 || !motionOwner(L) || !lua_isboolean(L, 2)) return result(L, false);
    // The sidebar policy restates the mode on every tick; only a change is
    // painted, so an idle desktop is not composited twice a second.
    const bool tiling = lua_toboolean(L, 2);
    if (tiling != nativeTiling) {
        nativeTiling = tiling;
        damage();
    }
    return result(L, true);
}
static void pushBox(lua_State* L, const CBox& box) {
    lua_newtable(L);
    lua_pushnumber(L, box.x); lua_setfield(L, -2, "x");
    lua_pushnumber(L, box.y); lua_setfield(L, -2, "y");
    lua_pushnumber(L, box.w); lua_setfield(L, -2, "width");
    lua_pushnumber(L, box.h); lua_setfield(L, -2, "height");
}
static int placeTile(lua_State* L) {
    if (lua_gettop(L) != 4 || !selected(L) || !owned() || !restoring || relocationConfirmed || !nativeTiling ||
        !lua_isinteger(L, 2) || lua_tointeger(L, 2) != ownerPid || !lua_isnumber(L, 3) || !lua_isnumber(L, 4)) { lua_pushnil(L); return 1; }
    const Vector2D point{lua_tonumber(L, 3), lua_tonumber(L, 4)};
    const auto w = app.lock();
    const auto target = w->layoutTarget();
    const auto space = target ? target->space() : nullptr;
    if (!std::isfinite(point.x) || !std::isfinite(point.y) || !CBox{output->m_position, output->m_size}.containsPoint(point) ||
        !space || space->workspace() != output->m_activeWorkspace || !target->floating() || (w->m_state & Desktop::View::WINDOW_STATE_PINNED) || Fullscreen::controller()->isFullscreen(w)) { lua_pushnil(L); return 1; }
    const auto prior = w->geometricBox(Desktop::View::IGeometric::GEOMETRIC_GOAL);
    w->setBox(CBox{output->m_position.x - prior.w - 64, output->m_position.y - prior.h - 64, prior.w, prior.h});
    w->finishAnimation();
    space->remove(target);
    target->setFloating(false);
    space->move(target, point);
    w->finishAnimation();
    if (target->floating()) { lua_pushnil(L); return 1; }
    const auto box = w->geometricBox(Desktop::View::IGeometric::GEOMETRIC_GOAL);
    wanted = box;
    deadline = Time::steadyNow() + std::chrono::seconds(3);
    const auto next = gate.begin(Time::steadyNow());
    if (!next) { fail("tile-gate-begin"); lua_pushnil(L); return 1; }
    ticket = *next;
    contentListener.reset();
    contentListener = w->backend().m_events.commit.listen([current = ticket](bool) {
        if (!owned() || foreign || detached || current != ticket) return;
        gate.observe(current, content(), Time::steadyNow());
        if (gate.state() == Enzui::ContentReadiness::State::Changed) fail("tile-content-changed");
        damage();
    });
    pushBox(L, box);
    return 1;
}
