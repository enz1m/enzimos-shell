static bool shapedControl(PHLWINDOW w) {
    return barRegions.enabled && panelOwned(w) && w->backend().metadata().appID == "dev.enzui.ControlCenter";
}
struct PanelContour {
    PHLWINDOWREF window;
    WP<CWLSurfaceResource> surface;
    CRegion region;
};
static std::map<uintptr_t, PanelContour> panelContours;
static void prunePanelContours() {
    std::erase_if(panelContours, [](const auto& entry) {
        const auto& c = entry.second;
        return !shapedControl(c.window.lock()) || !c.surface || c.window->backend().surface() != c.surface.lock();
    });
}
class PanelShapePass final : public IPassElement {
    UP<Render::CRenderPass> contents;
    PHLWINDOWREF window;
    pid_t owner;
  public:
    PanelShapePass(UP<Render::CRenderPass> pass, PHLWINDOW w)
        : contents(std::move(pass)), window(w), owner(ownerPid) {}
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    bool disableSimplification() override { return true; }
    // It draws only inside the frame's damage; the panel's commits damage its
    // own box, so its frames need not repaint the whole output.
    bool requiresFullDamage() override { return false; }
    const char* passName() override { return "EnzControlShape"; }
    ePassElementType type() override { return EK_CUSTOM; }
    std::vector<UP<IPassElement>> draw() override {
        const auto w = window.lock();
        const auto surface = w ? w->backend().surface() : nullptr;
        if (owner != ownerPid || !shapedControl(w) || !surface || !surface->m_current.texture ||
            surface->m_current.inputIsInfinite || g_pHyprRenderer->m_renderData.pMonitor != output || contents->single()) return {};
        const auto saved = g_pHyprRenderer->m_renderData;
        if (contents->needsLiveBlur() || contents->needsPrecomputeBlur() || !saved.renderModif.modifs.empty()) return {};
        auto& contour = panelContours[uintptr_t(w.get())];
        if (contour.window != w || contour.surface.lock() != surface) contour = {w, surface, {}};
        if (!surface->m_current.input.empty()) contour.region = surface->m_current.input;
        // The panel's own shadow falls just outside its input shape.
        auto clip = contour.region.copy().expand(28).intersect(CBox{0, 0, surface->m_current.size.x, surface->m_current.size.y});
        const auto origin = w->position(Desktop::View::IGeometric::GEOMETRIC_CURRENT) - output->m_position;
        clip.translate(origin).scale(output->m_scale).intersect(saved.damage);
        if (clip.empty()) {
            surface->presentFeedback(Time::steadyNow(), w->m_monitor.lock(), true);
            return {};
        }
        const bool blur = output->m_blurFBShouldRender;
        auto& data = g_pHyprRenderer->m_renderData;
        data.noSimplify = true;
        contents->render(clip);
        data = saved;
        output->m_blurFBShouldRender = blur;
        return {};
    }
};
