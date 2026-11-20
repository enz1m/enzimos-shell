static PHLWORKSPACEREF wallpaperCurrent;
static PHLWORKSPACE wallpaperPrevious;
static Time::steady_tp wallpaperSwitchAt;
static uint64_t wallpaperSwitches = 0, wallpaperPaintCalls = 0, wallpaperFrames = 0;
static bool wallpaperSourceReady = false;

static SP<Render::ITexture> wallpaperTexture() {
    if (!output)
        return {};
    const auto& layers = output->m_layerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND];
    for (auto it = layers.rbegin(); it != layers.rend(); ++it) {
        auto layer = it->lock();
        if (!layer || !layer->mapped() || (layer->m_namespace != "enzui-wallpaper" && layer->m_namespace != "wallpaper") ||
            layer->m_monitor.lock() != output)
            continue;
        auto resource = layer->m_layerSurface.lock();
        auto surface = resource ? resource->m_surface.lock() : nullptr;
        if (surface && surface->m_current.texture)
            return surface->m_current.texture;
    }
    return {};
}

static void paintWorkspaceTransition() {
    // The previous workspace is held strongly so an emptied, destroyed workspace can still slide out;
    // release it as soon as the transition can no longer paint.
    if (wallpaperPrevious && (!output || !ownerAlive() || !wallpaperCurrent ||
                              std::chrono::duration<double>(Time::steadyNow() - wallpaperSwitchAt).count() > 1.0)) {
        wallpaperPrevious.reset();
        return;
    }
    if (!output || g_pHyprRenderer->m_renderData.pMonitor != output || !wallpaperPrevious || !wallpaperCurrent)
        return;
    auto before = wallpaperPrevious;
    auto after = wallpaperCurrent.lock();
    if (!before || !after || before == after || before->m_monitor != output || after->m_monitor != output) {
        wallpaperPrevious.reset();
        return;
    }
    ++wallpaperPaintCalls;
    const double elapsed = std::chrono::duration<double>(Time::steadyNow() - wallpaperSwitchAt).count();
    const bool native = before->m_renderOffset->isBeingAnimated() || after->m_renderOffset->isBeingAnimated();
    if (!native && elapsed >= .26) {
        wallpaperPrevious.reset();
        return;
    }
    const auto texture = wallpaperTexture();
    wallpaperSourceReady = bool(texture);
    if (!texture)
        return;
    const double width = output->m_size.x;
    const double height = output->m_size.y;
    if (width < 320 || height < 200)
        return;
    // Laid out in logical pixels; pass element boxes are physical.
    const auto px = [](CBox box) { return box.scale(output->m_scale); };
    const auto previousId = before->numberedID();
    const auto nextId = after->numberedID();
    if (!previousId || !nextId)
        return;
    const double direction = *previousId < *nextId ? 1.0 : -1.0;
    double oldX = before->m_renderOffset->value().x;
    double newX = after->m_renderOffset->value().x;
    if (!native && std::abs(oldX) < 1 && std::abs(newX) < 1) {
        const double t = std::clamp(elapsed / .26, 0.0, 1.0);
        const double eased = 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t);
        oldX = -direction * width * eased;
        newX = direction * width * (1.0 - eased);
    }
    const double separation = std::min({std::abs(oldX), std::abs(newX), 40.0}) / 40.0 * 18.0;
    oldX -= direction * separation / 2.0;
    newX += direction * separation / 2.0;
    g_pHyprRenderer->addPassElement(makeUnique<CRectPassElement>(CRectPassElement::SRectData{
        .box = px({0, 0, width, height}), .color = CHyprColor{0.035F, 0.065F, 0.11F, 1.F}, .round = 0}));
    g_pHyprRenderer->addPassElement(makeUnique<CTexPassElement>(CTexPassElement::SRenderData{
        .tex = texture, .box = px({oldX, 0, width, height})}));
    g_pHyprRenderer->addPassElement(makeUnique<CTexPassElement>(CTexPassElement::SRenderData{
        .tex = texture, .box = px({newX, 0, width, height})}));
    if (separation > 2) {
        const double seam = direction > 0 ? width + oldX + separation / 2.0 : width + newX + separation / 2.0;
        if (seam >= 0 && seam < width)
            g_pHyprRenderer->addPassElement(makeUnique<CRectPassElement>(CRectPassElement::SRectData{
                .box = px({seam - 1, 0, 2, height}), .color = CHyprColor{0.35F, 0.58F, 0.91F, 0.54F}, .round = 0}));
    }
    ++wallpaperFrames;
    damage();
}

static int wallpaperTransitionState(lua_State* L) {
    lua_newtable(L);
    lua_pushinteger(L, wallpaperSwitches); lua_setfield(L, -2, "switches");
    lua_pushinteger(L, wallpaperPaintCalls); lua_setfield(L, -2, "paint_calls");
    lua_pushinteger(L, wallpaperFrames); lua_setfield(L, -2, "frames");
    lua_pushboolean(L, wallpaperSourceReady); lua_setfield(L, -2, "source_ready");
    lua_pushboolean(L, ownerAlive()); lua_setfield(L, -2, "owner_alive");
    lua_pushboolean(L, bool(wallpaperCurrent)); lua_setfield(L, -2, "current");
    lua_pushboolean(L, bool(wallpaperPrevious)); lua_setfield(L, -2, "previous");
    return 1;
}
