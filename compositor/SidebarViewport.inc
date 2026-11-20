struct ViewportLease {
    PHLWINDOWREF window;
    PHLMONITORREF monitor;
    CBox original, applied;
};
static std::vector<ViewportLease> viewportLeases;
static std::optional<std::array<double, 4>> workAreaInsets;

static bool maximizedCandidate(PHLWINDOW w) {
    return w && w->mapped() && w->m_monitor == output && w->backend().pid() != ownerPid &&
           !w->backend().isX11() && !remote(w) && w->m_workspace &&
           w->m_workspace->type() == Workspace::eWorkspaceType::NORMAL &&
           Fullscreen::controller()->getFullscreenModes(w).internal == Fullscreen::FSMODE_MAXIMIZED;
}
static CBox maximizedClientBox(PHLWINDOW w, CBox box) {
    if (Fullscreen::controller()->getFullscreenModes(w).internal == Fullscreen::FSMODE_MAXIMIZED) {
        const auto reserved = w->getFullWindowReservedArea();
        box.x += reserved.topLeft.x; box.y += reserved.topLeft.y;
        box.w -= reserved.topLeft.x + reserved.bottomRight.x;
        box.h -= reserved.topLeft.y + reserved.bottomRight.y;
    }
    return box;
}
static CBox compositorWindowBox(PHLWINDOW w) {
    auto m = w->m_monitor.lock();
    CBox box{m->m_position, m->m_size};
    if (Fullscreen::controller()->getFullscreenModes(w).internal == Fullscreen::FSMODE_MAXIMIZED) {
        box.x += m->m_reservedArea.left(); box.y += m->m_reservedArea.top();
        box.w -= m->m_reservedArea.left() + m->m_reservedArea.right();
        box.h -= m->m_reservedArea.top() + m->m_reservedArea.bottom();
    }
    return maximizedClientBox(w, box);
}
static void clearWorkAreaViewport() {
    for (auto &lease : viewportLeases) {
        auto w = lease.window.lock();
        if (w && w->mapped() && !remote(w) && w->m_monitor == lease.monitor &&
            Fullscreen::controller()->getFullscreenModes(w).internal != Fullscreen::FSMODE_NONE &&
            w->geometricBox(Desktop::View::IGeometric::GEOMETRIC_GOAL) == lease.applied) {
            w->setBox(compositorWindowBox(w));
            w->sendWindowSize();
        }
    }
    viewportLeases.clear();
    workAreaInsets.reset();
}
static void updateWorkAreaViewport() {
    if (!workAreaInsets)
        return;
    if (!ownerAlive()) {
        clearWorkAreaViewport();
        return;
    }
    std::erase_if(viewportLeases, [](const auto &lease) { return !maximizedCandidate(lease.window.lock()); });
    const auto &i = *workAreaInsets;
    const CBox outer{output->m_position.x + i[0], output->m_position.y + i[1],
                      output->m_size.x - i[0] - i[2], output->m_size.y - i[1] - i[3]};
    if (outer.w < 80 || outer.h < 80)
        return;
    for (const auto &w : Desktop::windowState()->windows()) {
        if (!maximizedCandidate(w) || !w->m_workspace->visible())
            continue;
        const auto wanted = maximizedClientBox(w, outer);
        if (wanted.w < 1 || wanted.h < 1) continue;
        const auto goalBox = w->geometricBox(Desktop::View::IGeometric::GEOMETRIC_GOAL);
        auto lease = std::find_if(viewportLeases.begin(), viewportLeases.end(), [&](const auto &v) { return v.window == w; });
        const auto coreBox = compositorWindowBox(w);
        if (lease == viewportLeases.end()) {
            if (goalBox != coreBox)
                continue;
            viewportLeases.push_back({w, output, coreBox, goalBox});
            lease = std::prev(viewportLeases.end());
        } else if (goalBox != lease->applied) {
            if (goalBox != coreBox) {
                viewportLeases.erase(lease);
                continue;
            }
            lease->original = coreBox;
        }
        if (goalBox != wanted) {
            lease->applied = wanted;
            w->setBox(wanted);
            w->sendWindowSize();
        }
    }
}
static int setWorkAreaViewport(lua_State *L) {
    if (lua_gettop(L) != 6 || !motionOwner(L) || !lua_isboolean(L, 2))
        return result(L, false);
    std::array<double, 4> values;
    for (int index = 0; index < 4; ++index) {
        if (!lua_isnumber(L, index + 3)) return result(L, false);
        values[index] = lua_tonumber(L, index + 3);
        if (!std::isfinite(values[index]) || values[index] < 0 || values[index] > 512)
            return result(L, false);
    }
    if (!lua_toboolean(L, 2)) {
        clearWorkAreaViewport();
    } else {
        if (output->m_size.x - values[0] - values[2] < 80 || output->m_size.y - values[1] - values[3] < 80)
            return result(L, false);
        workAreaInsets = values;
        updateWorkAreaViewport();
    }
    return result(L, true);
}
static int railContext(lua_State *L) {
    if (lua_gettop(L) != 1 || !motionOwner(L)) { lua_pushnil(L); return 1; }
    lua_createtable(L, 0, 3);
    lua_pushnumber(L, std::chrono::duration<double>(Time::steadyNow().time_since_epoch()).count()); lua_setfield(L, -2, "now");
    lua_pushboolean(L, iconDrop.held); lua_setfield(L, -2, "held");
    lua_pushboolean(L, railSource() && Desktop::focusState()->window() == railSource()); lua_setfield(L, -2, "keyboard");
    return 1;
}
