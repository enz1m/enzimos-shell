static void wakePanelFrame(PHLWINDOW w) {
    if (auto surface = w->backend().surface()) {
        const auto now = Time::steadyNow();
        if (w->alphaNonZero()) surface->frame(now);
        else surface->presentFeedback(now, output.lock(), true);
    }
}
// A hidden retained panel must finish painting before it can be revealed.
// Keep its callbacks moving only during that bounded, shell-owned preparation.
static int panelFrame(lua_State *L) {
    const auto w = find(L);
    if (lua_gettop(L) != 3 || !lua_isinteger(L, 2) || lua_tointeger(L, 2) != ownerPid ||
        !lua_isboolean(L, 3) || !panelOwned(w) || w->backend().metadata().appID != "dev.enzui.ControlCenter")
        return result(L, false);
    if (lua_toboolean(L, 3)) {
        panelFrameWake = PanelFrameWake{w, Time::steadyNow() + std::chrono::milliseconds(120)};
        w->setSuspended(false);
        wakePanelFrame(w);
        g_pHyprRenderer->damageMonitor(output.lock());
    } else if (panelFrameWake && panelFrameWake->window == w)
        panelFrameWake.reset();
    return result(L, true);
}
static int railFrames(lua_State *L) {
    if (lua_gettop(L) != 2 || !lua_isinteger(L, 1) || lua_tointeger(L, 1) != ownerPid || !lua_isinteger(L, 2) ||
        lua_tointeger(L, 2) < 0 || lua_tointeger(L, 2) > 2000 || !ownerAlive())
        return result(L, false);
    railFramesUntil = Time::steadyNow() + std::chrono::milliseconds(lua_tointeger(L, 2));
    if (const auto w = railSource())
        wakePanelFrame(w);
    g_pHyprRenderer->damageMonitor(output.lock());
    return result(L, true);
}
static int inputAt(lua_State* L) {
    const auto w = find(L);
    if (lua_gettop(L) != 4 || !lua_isinteger(L, 2) || lua_tointeger(L, 2) != ownerPid ||
        !ownerAlive() || !w || !w->mapped() || w->m_monitor != output ||
        w->backend().pid() != ownerPid || remote(w) || !lua_isnumber(L, 3) || !lua_isnumber(L, 4)) {
        lua_pushnil(L); return 1;
    }
    const auto& name = w->backend().metadata().appID;
    if (name == "dev.enzui.Desktop" && barFullscreenSuppressed) return result(L, false);
    if (!barRegions.enabled) { lua_pushnil(L); return 1; }
    if (name != "dev.enzui.Desktop" && name != "dev.enzui.ControlCenter" && name != "dev.enzui.PluginPanel" &&
        name != "dev.enzui.NotificationCenter" && name != "dev.enzui.WorkspaceOverview" && name != "dev.enzui.QuickSearch" && name != "dev.enzui.Launcher") {
        lua_pushnil(L); return 1;
    }
    const Vector2D point{lua_tonumber(L, 3), lua_tonumber(L, 4)};
    if (!std::isfinite(point.x) || !std::isfinite(point.y)) { lua_pushnil(L); return 1; }
    const auto surface = w->backend().surface();
    return result(L, surface && w->geometricBox(Desktop::View::IGeometric::GEOMETRIC_GOAL).containsPoint(point) &&
        surface->at(point - w->position(Desktop::View::IGeometric::GEOMETRIC_GOAL), true).first != nullptr);
}
static void primeIslandInput(PHLWINDOW w) {
    if (!w || !barRegions.enabled || !ownerAlive() || w->m_monitor != output ||
        w->backend().pid() != ownerPid)
        return;
    const auto& name = w->backend().metadata().appID;
    if (name != "dev.enzui.ControlCenter" && name != "dev.enzui.PluginPanel" && name != "dev.enzui.NotificationCenter" &&
        name != "dev.enzui.WorkspaceOverview" && name != "dev.enzui.QuickSearch" && name != "dev.enzui.Launcher")
        return;
    if (const auto surface = w->backend().surface()) {
        surface->m_current.input.clear();
        surface->m_current.inputIsInfinite = false;
    }
}
static int motion(lua_State *L) {
    if (lua_gettop(L) != 3 || !motionOwner(L) || !lua_isinteger(L, 2) ||
        lua_tointeger(L, 2) < 25 || lua_tointeger(L, 2) > 200 || !lua_isboolean(L, 3))
        return result(L, false);
    const auto rate = static_cast<int>(lua_tointeger(L, 2));
    if (rate != motionRate && restoreFrame && restoreFrame->started) {
        updateRestoreProgress(Time::steadyNow());
        restoreFrame->from = restoreFrame->progress;
        restoreFrame->began = Time::steadyNow();
    }
    if (rate != motionRate && animating) {
        const auto now = Time::steadyNow();
        const auto elapsed = std::chrono::duration<double>(now - began).count();
        progress = appProgress(elapsed);
        from = progress;
        began = now;
        animating = progress != goal;
    }
    if (rate != motionRate) {
        const auto now = Time::steadyNow();
        samplePocket(now);
        pocket.from = pocket.progress;
        pocket.began = now;
        sampleRail(now);
        rail.from = rail.progress;
        rail.began = now;
    }
    motionRate = rate;
    windowMotion = lua_toboolean(L, 3);
    damage();
    return result(L, true);
}
static int motionState(lua_State *L) {
    if (lua_gettop(L) != 1 || !motionOwner(L)) {
        lua_pushnil(L);
        return 1;
    }
    lua_newtable(L);
    lua_pushinteger(L, motionRate);
    lua_setfield(L, -2, "rate");
    lua_pushboolean(L, windowMotion);
    lua_setfield(L, -2, "windows");
    return 1;
}
static int version(lua_State *L) {
    lua_pushinteger(L, 8);
    return 1;
}
static int ownerAliveApi(lua_State *L) {
    return result(L, lua_isinteger(L, 1) && lua_tointeger(L, 1) == ownerPid && ownerAlive());
}
static int configure(lua_State *L) {
    if (!lua_isinteger(L, 1) || !lua_isstring(L, 2) || lua_tointeger(L, 1) <= 1 ||
        lua_tointeger(L, 1) > INT32_MAX)
        return result(L, false);
    const auto pid = lua_tointeger(L, 1);
    if (active && (pid != ownerPid || !output || output->m_name != lua_tostring(L, 2)))
        return result(L, false);
    PHLWINDOW found;
    for (auto &w : Desktop::windowState()->windows())
        if (w->mapped() && w->backend().pid() == pid &&
            w->backend().metadata().appID == "dev.enzui.Desktop" && w->m_monitor &&
            w->m_monitor->m_name == lua_tostring(L, 2)) {
            if (found)
                return result(L, false);
            found = w;
        }
    if (!found || remote(found))
        return result(L, false);
    if (ownerPid != pid || output != found->m_monitor) {
        searchDim = {};
        clearWorkAreaViewport();
        resetBarRegions();
        pocket = {};
        clearRail();
        panelAnchors.clear();
        panelFrameWake.reset();
        railFramesUntil.reset();
    }
    ownerPid = pid;
    bar = found;
    output = found->m_monitor;
    return result(L, true);
}
static int setRail(lua_State *L) {
    if ((lua_gettop(L) < 9 || lua_gettop(L) > 11) || !motionOwner(L) || !lua_isboolean(L, 2) || !lua_isboolean(L, 9) ||
        (lua_gettop(L) >= 10 && !lua_isboolean(L, 10)) || (lua_gettop(L) == 11 && !lua_isboolean(L, 11)))
        return result(L, false);
    for (int i = 3; i <= 8; ++i)
        if (!lua_isnumber(L, i) || !std::isfinite(lua_tonumber(L, i)))
            return result(L, false);
    CBox box{lua_tonumber(L, 3), lua_tonumber(L, 4), lua_tonumber(L, 5), lua_tonumber(L, 6)};
    box.translate(-output->m_position);
    const double solid = lua_tonumber(L, 7), frame = lua_tonumber(L, 8);
    const bool onRight = lua_toboolean(L, 9);
    const bool floating = lua_gettop(L) >= 10 && lua_toboolean(L, 10);
    const bool clientShape = lua_gettop(L) == 11 && lua_toboolean(L, 11);
    if (clientShape && !floating)
        return result(L, false);
    const double edge = onRight ? output->m_size.x - box.w : 0;
    const double inset = onRight ? edge - box.x : box.x;
    if ((floating ? (!islandsSupported() || inset < 4 || inset > 48 || box.w != solid || frame != 0) : box.x != edge) ||
        box.y < 0 || box.w < solid || box.w > solid + 128 || box.h < 1 ||
        box.y + box.h > output->m_size.y || solid < 16 || solid > 256 || frame < 0 || frame > solid)
        return result(L, false);
    const auto now = Time::steadyNow();
    const double next = lua_toboolean(L, 2) ? 1 : 0;
    sampleRail(now);
    const bool changed = !rail.configured || rail.box != box || rail.solid != solid ||
                         rail.frame != frame || rail.right != onRight || rail.goal != next || rail.floating != floating || rail.clientShape != clientShape;
    rail.box = box;
    rail.solid = solid;
    rail.frame = frame;
    rail.right = onRight;
    rail.floating = floating;
    rail.clientShape = clientShape;
    rail.configured = true;
    if (rail.goal != next) {
        rail.from = rail.progress;
        rail.goal = next;
        rail.began = now;
    }
    if (changed)
        damage();
    return result(L, true);
}
static int takeIconDrop(lua_State *L) {
    if (lua_gettop(L) != 3 || !motionOwner(L) || !lua_isnumber(L, 2) || !lua_isnumber(L, 3))
        return result(L, false);
    const bool qualified = iconDropOwner == ownerPid && iconDropOutput == output && iconDropSource && iconDropSource == railSource();
    const double now = std::chrono::duration<double>(Time::steadyNow().time_since_epoch()).count();
    return result(L, iconDrop.take(qualified, now, lua_tonumber(L, 2), lua_tonumber(L, 3)));
}
static int cancelIconDrop(lua_State *L) {
    if (lua_gettop(L) != 1 || !motionOwner(L))
        return result(L, false);
    iconDrop.cancel();
    pinDrag = {};
    if (iconFeedback.active) damage();
    return result(L, true);
}
static int railState(lua_State *L) {
    if (lua_gettop(L) != 1 || !motionOwner(L)) { lua_pushnil(L); return 1; }
    lua_newtable(L);
    lua_pushnumber(L, rail.progress); lua_setfield(L, -2, "progress");
    lua_pushnumber(L, rail.goal); lua_setfield(L, -2, "goal");
    lua_pushnumber(L, railShift().x); lua_setfield(L, -2, "shift");
    lua_pushboolean(L, railReady()); lua_setfield(L, -2, "ready");
    lua_pushboolean(L, railInputReady()); lua_setfield(L, -2, "input");
    lua_pushboolean(L, rail.floating); lua_setfield(L, -2, "floating");
    lua_pushboolean(L, rail.clientShape); lua_setfield(L, -2, "client_shape");
    return 1;
}
static int anchorPanel(lua_State *L) {
    const auto w = find(L);
    if ((lua_gettop(L) != 6 && lua_gettop(L) != 7) || !lua_isinteger(L, 2) || lua_tointeger(L, 2) != ownerPid || !panelOwned(w))
        return result(L, false);
    const bool bar = w->backend().metadata().appID == "dev.enzui.Desktop";
    // A popover under a top bar names its fixed edge too: it keeps its top
    // while its height follows what it holds.
    if (lua_gettop(L) == 7 && !lua_isboolean(L, 7))
        return result(L, false);
    const bool bottom = lua_gettop(L) == 6 || lua_toboolean(L, 7);
    for (int i = 3; i <= 6; ++i)
        if (!lua_isnumber(L, i) || !std::isfinite(lua_tonumber(L, i)))
            return result(L, false);
    CBox target{lua_tonumber(L, 3), lua_tonumber(L, 4), lua_tonumber(L, 5), lua_tonumber(L, 6)};
    const auto local = target.copy().translate(-output->m_position);
    if (!Enzui::panelAnchorFits(local.x, local.y, local.w, local.h, output->m_size.x, output->m_size.y, bar))
        return result(L, false);
    std::erase_if(panelAnchors, [&](const auto &anchor) { return !panelOwned(anchor.window.lock()) || anchor.window == w; });
    if (panelAnchors.size() >= 8)
        return result(L, false);
    const auto current = w->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT);
    panelAnchors.push_back(PanelAnchor{.window = w, .target = target, .guard = current, .initialX = current.x, .bottom = bottom});
    damage();
    return result(L, true);
}
static int panelState(lua_State *L) {
    const auto w = find(L);
    if (!lua_isinteger(L, 2) || lua_tointeger(L, 2) != ownerPid || !panelOwned(w)) { lua_pushnil(L); return 1; }
    for (const auto &anchor : panelAnchors) {
        if (anchor.window != w)
            continue;
        lua_newtable(L);
        lua_pushboolean(L, anchor.pending); lua_setfield(L, -2, "pending");
        lua_pushinteger(L, anchor.moves); lua_setfield(L, -2, "moves");
        lua_pushnumber(L, anchor.target.y + anchor.target.h); lua_setfield(L, -2, "bottom");
        return 1;
    }
    lua_pushnil(L);
    return 1;
}
static int clientResizeDisabled(lua_State *L) {
    auto w = find(L);
    lua_pushboolean(L, w && w->m_ruleApplicator && w->m_ruleApplicator->noXdgDrags().valueOrDefault());
    return 1;
}
static int supports(lua_State *L) {
    auto w = find(L);
    if (!ownerAlive() || !lua_isinteger(L, 2) || lua_tointeger(L, 2) != ownerPid || !supported(w) ||
        w->m_monitor != output || w->isInputBlocked() || w->popupMappedTreeSize() != 0)
        return result(L, false);
    const auto box = w->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT);
    watch(w);
    auto surface = w->backend().surface();
    return result(L, qualifications.contains(uintptr_t(w.get())) && box.w >= 80 && box.h >= 80 &&
                         box.w <= 8192 && box.h <= 8192 && surface && surface->m_current.texture &&
                         geometryOf(w).matches(box.w, box.h));
}
static int setPocket(lua_State *L) {
    if (lua_gettop(L) != 8 || !motionOwner(L) || !lua_isboolean(L, 2) ||
        !lua_isboolean(L, 3) || !lua_isboolean(L, 8))
        return result(L, false);
    const bool show = lua_toboolean(L, 2);
    if (!show) {
        pocket.requested = false;
        if (!pocket.handoff)
            pocketGoal(0);
        return result(L, true);
    }
    for (int i = 4; i <= 7; ++i)
        if (!lua_isnumber(L, i) || !std::isfinite(lua_tonumber(L, i)))
            return result(L, false);
    CBox box{lua_tonumber(L, 4), lua_tonumber(L, 5), lua_tonumber(L, 6), lua_tonumber(L, 7)};
    box.translate(-output->m_position);
    if (box.x < 0 || box.y < 0 || box.w < 80 || box.h < 80 || box.w > 512 || box.h > 1024 ||
        box.x + box.w > output->m_size.x || box.y + box.h > output->m_size.y || !validPocketDrag())
        return result(L, false);
    if (pocket.box != box)
        pocket = {};
    pocket.box = box;
    pocket.right = lua_toboolean(L, 8);
    pocket.requested = true;
    pocket.hover = lua_toboolean(L, 3);
    pocket.lease = Time::steadyNow() + std::chrono::seconds(1);
    if (!pocket.handoff)
        pocketGoal(pocket.hover ? 0.36 : 1.0 / 3.0);
    return result(L, true);
}
static int holdPocketApi(lua_State *L) {
    if (lua_gettop(L) != 1 || !motionOwner(L) || !owned() || !detached || foreign || restoring || additionalWindow())
        return result(L, false);
    holdPocket();
    return result(L, true);
}
static int pocketState(lua_State *L) {
    if (!motionOwner(L)) { lua_pushnil(L); return 1; }
    lua_newtable(L);
    lua_pushnumber(L, pocket.progress); lua_setfield(L, -2, "progress");
    lua_pushnumber(L, pocket.goal); lua_setfield(L, -2, "goal");
    lua_pushboolean(L, pocket.handoff); lua_setfield(L, -2, "handoff");
    lua_pushnumber(L, rail.floating ? lastOpenFrom : pocket.openedFrom); lua_setfield(L, -2, "opened_from");
    lua_pushstring(L, failureReason); lua_setfield(L, -2, "failure");
    return 1;
}
static int begin(lua_State *L, bool restore, bool repin = false) {
    if (!lua_isinteger(L, 2) || lua_tointeger(L, 2) != ownerPid || (!restore && !ownerAlive()))
        return result(L, false);
    for (int i = 3; i <= 6; ++i)
        if (!lua_isnumber(L, i) || !std::isfinite(lua_tonumber(L, i)))
            return result(L, false);
    CBox box{lua_tonumber(L, 3), lua_tonumber(L, 4), lua_tonumber(L, 5), lua_tonumber(L, 6)};
    if (std::abs(box.x) > 100000 || std::abs(box.y) > 100000 || box.w < 80 || box.h < 80 ||
        box.w > 8192 || box.h > 8192 || box.w != std::floor(box.w) || box.h != std::floor(box.h) ||
        (!restore && !lua_isboolean(L, 7)))
        return result(L, false);
    const bool explicitInset = !restore && !lua_isnoneornil(L, 8);
    const double inset = explicitInset ? lua_tonumber(L, 8) : 6;
    if ((explicitInset && !lua_isinteger(L, 8)) || !Enzui::validPaneInset(!restore && rail.floating, inset))
        return result(L, false);
    std::optional<Vector2D> anchor;
    if (!restore && !repin && !lua_isnoneornil(L, 9)) {
        if (!rail.floating || !lua_isnumber(L, 9) || !lua_isnumber(L, 10))
            return result(L, false);
        const Vector2D point{lua_tonumber(L, 9), lua_tonumber(L, 10)};
        if (!std::isfinite(point.x) || !std::isfinite(point.y) ||
            !CBox{output->m_position, output->m_size}.containsPoint(point))
            return result(L, false);
        anchor = point - output->m_position;
    }
    auto w = find(L);
    if (repin && (!active || !detached || !selected(L) || !owned() || foreign || restoring || additionalWindow()))
        return result(L, false);
    if (active && detached && draggingApp())
        return result(L, false);
    if (!w || !w->mapped() || w->m_monitor != output || remote(w) || w->backend().isX11() ||
        (active && (!selected(L) || (!restore && !repin))) || (!restore && !supported(w)))
        return result(L, false);
    if (!active && w->isInputBlocked())
        return result(L, false);
    std::optional<SmallFrame> retained;
    if (restore && active && !foreign && !additionalWindow() && rasterSupported() &&
        box.w * box.h <= 8388608) {
        if (detached && smallCommitted.image && smallCommitted.window == w &&
            smallCommitted.epoch == ticket && smallCommitted.box.size() == wanted.size() &&
            w->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT).size() == wanted.size())
            retained = std::move(smallCommitted);
        else if (restoring && failed && restoreFrame && restoreFrame->source.window == w)
            retained = std::move(restoreFrame->source);
    }
    clearPictures();
    if (retained)
        restoreFrame = RestoreFrame{.source = std::move(*retained)};
    if (restoreFrame)
        restoreFrame->visible = restoreFrame->source.box;
    app = w;
    if (!repin)
        iconAnchor = anchor;
    wanted = box;
    pane = box.copy().translate(-output->m_position).expand(inset);
    if (repin && pocket.box == pane) {
        holdPocket();
        pocket.lease = Time::steadyNow() + std::chrono::seconds(3);
    }
    resizeReattach = repin && resizeReattach && !failed && !foreign;
    resizeBackdropHeld = repin && resizeBackdropHeld;
    active = true;
    restoring = restore;
    relocationConfirmed = false;
    detached = desktopRequired = false;
    right = !restore && lua_toboolean(L, 7);
    failed = foreign = closed = closingArmed = openingArmed = animating = started = false;
    failureReason = "none";
    progress = from = 0;
    goal = 1;
    began = Time::steadyNow();
    deadline = began + std::chrono::seconds(3);
    const auto next = gate.begin(began);
    if (!next) {
        fail("gate-begin");
        return result(L, false);
    }
    ticket = *next;
    block();
    watch(w);
    auto q = qualifications.find(uintptr_t(w.get()));
    if (q != qualifications.end() && q->second.matches(w, wanted.size())) {
        // The cache identifies an applied attachment; pending or metadata-only
        // state cannot seed it.
        gate.observe(ticket, {true, true, true, true, true}, began);
        ++reused;
    }
    contentListener.reset();
    contentListener = w->backend().m_events.commit.listen([current = ticket](bool) {
        if (!owned() || foreign || detached || current != ticket)
            return;
        if (resizingApp()) {
            refresh();
            if (detached || failed)
                return;
        }
        gate.observe(current, content(), Time::steadyNow());
        if (gate.state() == Enzui::ContentReadiness::State::Changed)
            fail("committed-content-changed");
        damage();
    });
    commitListener.reset();
    commitListener = output->m_output->events.commit.listen([] {
        if (!owned() || foreign)
            return;
        const auto &state = output->m_output->state->state();
        if (!(state.committed & Aquamarine::COutputState::AQ_OUTPUT_STATE_BUFFER) || !state.buffer)
            return;
        if (smallPending && !restoring && smallScratch) {
            auto spare = std::move(smallCommitted.image);
            smallCommitted = SmallFrame{.image = std::move(smallScratch), .window = app,
                                        .box = smallPendingBox, .epoch = ticket};
            smallScratch = std::move(spare);
            smallPending = false;
        }
        if (closingArmed && goal == 0 && progress == 0)
            closed = true;
        // An open app re-arms this on every frame it is drawn; only the
        // frame that ends the opening unblocks it and re-probes the pointer.
        // Probing on every frame sent a pointer motion per frame, whose
        // damage drew the next frame: the output repainted at its full rate
        // for as long as a pinned app was open.
        if (openingArmed && gate.ready() && !failed) {
            openingArmed = false;
            if (blocked) {
                unblock();
                g_pInputManager->simulateMouseMovement();
            }
        }
    });
    damage();
    return result(L, true);
}
static int prepare(lua_State *L) { return begin(L, false); }
static int restore(lua_State *L) { return begin(L, true); }
static int repin(lua_State *L) { return begin(L, false, true); }
static int relocated(lua_State *L) {
    if (!selected(L) || !owned() || !restoring || foreign || !lua_isinteger(L, 2) ||
        lua_tointeger(L, 2) != ownerPid)
        return result(L, false);
    relocationConfirmed = true;
    damage();
    return result(L, true);
}
static int move(lua_State *L, double next) {
    if (!selected(L) || !owned() || foreign || restoring || detached)
        return result(L, false);
    if (goal == next)
        return result(L, true);
    block();
    closed = closingArmed = false;
    from = progress;
    goal = next;
    began = Time::steadyNow();
    animating = gate.ready() && !failed;
    if (!animating && next == 0)
        progress = from = 0;
    damage();
    return result(L, true);
}
static int show(lua_State *L) { return move(L, 1); }
static int hide(lua_State *L) { return move(L, 0); }
static const char *phase() {
    refresh();
    if (!active || !app || !app->mapped())
        return "idle";
    if (foreign)
        return "foreign";
    if (detached)
        return draggingApp() ? "dragging" : "detached";
    if (desktopRequired)
        return "dialog";
    if (failed && !blocked && restoreOnDesktop())
        return "restored";
    if (failed)
        return "failed";
    if (goal == 0)
        return closed ? "closed" : "closing";
    if (restoring)
        return restoredContentReady() && !blocked ? "restored" : "restoring";
    return started && progress == 1 && !blocked ? "open" : "opening";
}
static int status(lua_State *L) {
    const auto p = phase();
    lua_pushstring(L, selected(L) ? p : "idle");
    return 1;
}
static int finish(lua_State *L) {
    const auto p = std::string_view(phase());
    const bool cancelledRestore = p == "failed" && restoring && !blocked;
    if (app && app->mapped() &&
        (!selected(L) || (p != "closed" && p != "restored" && p != "foreign" && !cancelledRestore)))
        return result(L, false);
    clear();
    return result(L, true);
}
static int counters(lua_State *L) {
    lua_pushinteger(L, rendered);
    lua_pushinteger(L, reused);
    lua_pushinteger(L, qualifications.size());
    return 3;
}
static int surfaceState(lua_State *L) {
    auto w = find(L);
    if (!w || !w->mapped() || remote(w) || w->backend().pid() == ownerPid || !lua_isinteger(L, 2) ||
        lua_tointeger(L, 2) != ownerPid)
        return 0;
    size_t count = 0;
    bool pointer = false, keyboard = false;
    surfaces(w, [&](SP<CWLSurfaceResource> s) {
        ++count;
        pointer |= g_pSeatManager->m_state.pointerFocus == s;
        keyboard |= g_pSeatManager->m_state.keyboardFocus == s;
    });
    lua_pushinteger(L, count);
    lua_pushinteger(L, w->popupMappedTreeSize());
    lua_pushinteger(L, dismissed);
    lua_pushinteger(L, pointer);
    lua_pushinteger(L, keyboard);
    return 5;
}
