struct OverviewBackdrop {
    PHLWINDOWREF window;
    PHLWORKSPACEREF workspace;
    double progress = 0, from = 0, goal = 0;
    bool painted = false;
    Time::steady_tp began;
};
static OverviewBackdrop overviewBackdrop;

static void tickOverviewBackdrop() {
    auto& state = overviewBackdrop;
    state.painted = false;
    if (!ownerAlive() || !output->enabled() || (state.workspace && state.workspace != output->m_activeWorkspace)) {
        if (state.progress > 0) damage();
        state = {};
        return;
    }
    PHLWINDOW overview;
    for (const auto& w : Desktop::windowState()->windows()) {
        if (w->mapped() && !w->isHidden() && w->backend().pid() == ownerPid &&
            w->backend().metadata().appID == "dev.enzui.WorkspaceOverview" &&
            w->m_monitor == output && w->m_workspace == output->m_activeWorkspace && !remote(w)) {
            overview = w;
            break;
        }
    }
    const double goal = overview ? 1 : 0;
    const auto now = Time::steadyNow();
    if (goal != state.goal) {
        state.from = state.progress;
        state.goal = goal;
        state.began = now;
        state.window = overview;
        state.workspace = output->m_activeWorkspace;
    }
    const auto before = state.progress;
    state.progress = windowMotion ? Enzui::sceneProgress(state.from, state.goal,
        std::chrono::duration<double>(now - state.began).count(), motionRate) : state.goal;
    if (state.progress != before || state.progress != state.goal) damage();
}

static void paintOverviewBackdrop(eRenderStage stage) {
    auto& state = overviewBackdrop;
    if (state.painted || state.progress <= 0 || !ownerAlive() ||
        g_pHyprRenderer->m_renderData.pMonitor != output || g_pHyprRenderer->m_bRenderingSnapshot || redirect)
        return;
    if (!workspaceFullscreenPainted && output->m_activeWorkspace &&
        Fullscreen::controller()->hasFullscreen(output->m_activeWorkspace)) return;
    if (stage != RENDER_POST_WINDOWS && !(stage == RENDER_PRE_WINDOW &&
        g_pHyprRenderer->m_renderData.currentWindow == state.window)) return;
    // A uniform scrim over the desktop: windows stay where they are,
    // recognisably dimmed, under the desktops strip; the bar and the strip
    // are layers above it. (Veiling windows with the wallpaper left half-faded
    // "ghost" windows; stopping at the reserved band left an undimmed strip
    // behind the islands bar.) No client geometry, workspace, focus or saved
    // opacity is changed.
    // The frame's own chrome (bar, rail and edges: the reserved area) stays
    // as it is; with islands the band behind the bar is wallpaper and dims
    // with the rest (stopping there left an undimmed strip).
    // That band can be any edge's: extending only upward left a bottom bar's
    // band undimmed.
    auto box = barRegions.enabled ? output->logicalBox() : output->logicalBoxMinusReserved();
    box.translate(-output->logicalBox().pos());
    box.scale(output->m_scale);
    CRectPassElement::SRectData data;
    data.box = box.round();
    data.color = CHyprColor(0, 0, 0, 0.55 * state.progress);
    g_pHyprRenderer->addPassElement(makeUnique<CRectPassElement>(data));
    state.painted = true;
}
