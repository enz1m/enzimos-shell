struct SearchDim {
    PHLWINDOWREF window;
    PHLWORKSPACEREF workspace;
    uint64_t epoch = 0;
    double progress = 0, from = 0, goal = 0;
    bool fade = true, painted = false;
    Time::steady_tp began;
};
static SearchDim searchDim;
static void sampleSearchDim(Time::steady_tp now) {
    searchDim.progress = searchDim.fade
        ? Enzui::searchDimProgress(searchDim.from, searchDim.goal,
            std::chrono::duration<double>(now - searchDim.began).count())
        : searchDim.goal;
}
static void searchDimGoal(double goal, bool fade) {
    const auto now = Time::steadyNow();
    sampleSearchDim(now);
    if (searchDim.goal == goal && searchDim.fade == fade) return;
    searchDim.from = searchDim.progress;
    searchDim.goal = goal;
    searchDim.fade = fade;
    searchDim.began = now;
    sampleSearchDim(now);
    damage();
}
static void tickSearchDim() {
    searchDim.painted = false;
    if (!ownerAlive() || !output->enabled() || searchDim.workspace != output->m_activeWorkspace) {
        if (searchDim.progress > 0) damage();
        searchDim = {};
        return;
    }
    if (searchDim.goal > 0 && (!searchDim.window || !searchDim.window->mapped() ||
        searchDim.window->m_workspace != searchDim.workspace || searchDim.window->m_monitor != output))
        searchDimGoal(0, searchDim.fade);
    const auto previous = searchDim.progress;
    sampleSearchDim(Time::steadyNow());
    if (searchDim.progress != searchDim.goal || searchDim.progress != previous) damage();
}
static void paintSearchDim(eRenderStage stage) {
    if (searchDim.painted || searchDim.progress <= 0 || !ownerAlive() ||
        searchDim.workspace != output->m_activeWorkspace ||
        g_pHyprRenderer->m_renderData.pMonitor != output || g_pHyprRenderer->m_bRenderingSnapshot || redirect)
        return;
    if (!workspaceFullscreenPainted && output->m_activeWorkspace &&
        Fullscreen::controller()->hasFullscreen(output->m_activeWorkspace)) return;
    if (stage != RENDER_POST_WINDOWS && !(stage == RENDER_PRE_WINDOW &&
        g_pHyprRenderer->m_renderData.currentWindow == searchDim.window)) return;
    static auto dim = CConfigValue<Config::FLOAT>("decoration:dim_around");
    CRectPassElement::SRectData data;
    data.box = {0, 0, output->m_transformedSize.x, output->m_transformedSize.y};
    data.color = CHyprColor(0, 0, 0, *dim * searchDim.progress);
    g_pHyprRenderer->addPassElement(makeUnique<CRectPassElement>(data));
    searchDim.painted = true;
}
static int setSearchDim(lua_State* L) {
    if (lua_gettop(L) != 5 || !motionOwner(L) || !lua_isstring(L, 2) ||
        !lua_isinteger(L, 3) || lua_tointeger(L, 3) < 0 ||
        !lua_isboolean(L, 4) || !lua_isboolean(L, 5)) return result(L, false);
    PHLWINDOW window;
    for (const auto& w : Desktop::windowState()->windows())
        if (address(w) == lua_tostring(L, 2)) window = w;
    if (!window || !window->mapped() || window->backend().pid() != ownerPid ||
        window->backend().metadata().appID != "dev.enzui.QuickSearch" ||
        window->m_monitor != output || remote(window)) return result(L, false);
    const auto epoch = static_cast<uint64_t>(lua_tointeger(L, 3));
    if (epoch < searchDim.epoch) return result(L, false);
    if (lua_toboolean(L, 4)) {
        if (window->m_workspace != output->m_activeWorkspace) return result(L, false);
        searchDim.workspace = output->m_activeWorkspace;
    }
    searchDim.window = window;
    searchDim.epoch = epoch;
    searchDimGoal(lua_toboolean(L, 4) ? 1 : 0, lua_toboolean(L, 5));
    return result(L, true);
}
