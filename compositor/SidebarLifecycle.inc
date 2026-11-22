static void init(HANDLE handle) {
    rendererSlot = static_cast<RendererSlot *>(dlsym(RTLD_DEFAULT, "g_pHyprRenderer"));
    seatSlot = static_cast<SeatSlot *>(dlsym(RTLD_DEFAULT, "g_pSeatManager"));
    inputSlot = static_cast<InputSlot *>(dlsym(RTLD_DEFAULT, "g_pInputManager"));
    layoutSlot = static_cast<LayoutSlot *>(dlsym(RTLD_DEFAULT, "g_layoutManager"));
    // Resolve the pinned compositor's controller, not this plugin's inline copy.
    cursorSlot = reinterpret_cast<CursorSlot>(dlsym(RTLD_DEFAULT, "_ZN7Pointer6Cursor18overrideControllerE"));
    if (!rendererSlot || !*rendererSlot || !seatSlot || !*seatSlot || !inputSlot || !*inputSlot ||
        !layoutSlot || !*layoutSlot || !cursorSlot || !*cursorSlot)
        throw std::runtime_error("Native sidebar host symbols unavailable");
    // Launcher admission enables the provider; configure() still verifies shell-window ownership.
    const auto preview = std::getenv("ENZUI_SHELL_PLATFORM_PREVIEW");
    const auto session = std::getenv("ENZUI_DESKTOP_SESSION");
    const bool launched = (preview && std::string_view(preview) == "1") ||
                          (session && (std::string_view(session) == "installed" || std::string_view(session) == "preview"));
    if (!launched)
        throw std::runtime_error("Native sidebar requires an enzUI desktop launcher (ENZUI_DESKTOP_SESSION)");
    for (auto [name, fn] :
         {std::pair{"version", version}, std::pair{"configure", configure},
          std::pair{"search_dim", setSearchDim}, std::pair{"motion", motion}, std::pair{"motion_state", motionState},
          std::pair{"supports_islands", islandsSupportedApi}, std::pair{"supports_dock", dockSupportedApi}, std::pair{"bar_mode", barMode},
          std::pair{"bar_regions", setBarRegions}, std::pair{"bar_regions_state", barRegionsState}, std::pair{"input_at", inputAt}, std::pair{"panel_frame", panelFrame}, std::pair{"rail_frames", railFrames},
          std::pair{"rail", setRail}, std::pair{"rail_state", railState},
          std::pair{"icon_drop", takeIconDrop}, std::pair{"cancel_icon_drop", cancelIconDrop}, std::pair{"icon_feedback", beginIconFeedback}, std::pair{"drag_active", pinDragActive}, std::pair{"drag_drop", pinDragDrop}, std::pair{"drag_state", pinDragState},
          std::pair{"tiling_mode", tilingMode}, std::pair{"place_tile", placeTile},
          std::pair{"fullscreen_area", setWorkAreaViewport}, std::pair{"rail_context", railContext},
          std::pair{"anchor_panel", anchorPanel}, std::pair{"panel_state", panelState},
          std::pair{"pocket", setPocket}, std::pair{"pocket_state", pocketState},
          std::pair{"hold_pocket", holdPocketApi}, std::pair{"repin", repin},
          std::pair{"supports", supports}, std::pair{"size_limits", sizeLimits}, std::pair{"owner_alive", ownerAliveApi},
          std::pair{"prepare", prepare}, std::pair{"restore", restore},
          std::pair{"relocated", relocated}, std::pair{"show", show}, std::pair{"hide", hide},
          std::pair{"status", status}, std::pair{"client_resize_disabled", clientResizeDisabled},
          std::pair{"wallpaper_transition_state", wallpaperTransitionState}, std::pair{"finish", finish}, std::pair{"counters", counters},
          std::pair{"surface_state", surfaceState}})
        if (!HyprlandAPI::addLuaFunction(handle, "enzui_sidebar_scene", name, fn))
            throw std::runtime_error("Cannot register native sidebar provider");
    pointerFocusListener = g_pSeatManager->m_events.pointerFocusChange.listen(clearBlockedFocus);
    keyboardFocusListener = g_pSeatManager->m_events.keyboardFocusChange.listen(clearBlockedFocus);
    openListener = Event::bus()->m_events.window.open.listen([](PHLWINDOW w) { watch(w); });
    for (const auto &w : Desktop::windowState()->windows())
        if (w->mapped())
            watch(w);
    workspaceSwitchListener = Event::bus()->m_events.workspace.active.listen([](PHLWORKSPACE workspace) {
        if (!output || !ownerAlive() || !workspace || workspace->m_monitor != output)
            return;
        if (wallpaperCurrent && wallpaperCurrent != workspace) {
            ++wallpaperSwitches;
            wallpaperPrevious = wallpaperCurrent.lock();
            wallpaperSwitchAt = Time::steadyNow();
            damage();
        }
        wallpaperCurrent = workspace;
    });
    buttonListener = Event::bus()->m_events.input.mouse.button.listen(
        [](const IPointer::SButtonEvent &event, Event::SCallbackInfo &info) {
            const auto point = g_pInputManager->getMouseCoordsInternal();
            if (event.button == 272 && !event.state && chromeHeld) {
                chromeHeld = false;
                g_layoutManager->endDragTarget();
                updateChromeCursor();
                info.cancelled = true;
                return;
            }
            if (event.button == 272 && event.state && !g_layoutManager->dragController()->target() &&
                (detachTab().containsPoint(point) || resizeGrip().containsPoint(point))) {
                const bool resize = resizeGrip().containsPoint(point);
                g_layoutManager->beginDragTarget(app->layoutTarget(), resize ? MBIND_RESIZE : MBIND_MOVE,
                                                 resize ? std::optional<Layout::eRectCorner>{right ? Layout::CORNER_BOTTOMLEFT : Layout::CORNER_BOTTOMRIGHT} : std::nullopt, true);
                chromeHeld = bool(g_layoutManager->dragController()->target());
                if (chromeHeld) {
                    updateChromeCursor();
                    info.cancelled = true;
                    return;
                }
            }
            if (event.button == 272) {
                if (event.state != 0) {
                    iconDrop.press(ownerAlive() && railInputReady() && railPoint(point), point.x, point.y);
                    iconDropSource = railSource();
                    iconDropOutput = output;
                    iconDropOwner = ownerPid;
                } else {
                    iconDrop.release(std::chrono::duration<double>(Time::steadyNow().time_since_epoch()).count(), point.x, point.y);
                    if (iconFeedback.active) damage();
                }
            } else if (event.state != 0) {
                iconDrop.cancel();
            }
            if (event.button >= suppressed.size()) {
                if (active && blocked)
                    info.cancelled = true;
                return;
            }
            if (event.state == 0 && suppressed.test(event.button)) {
                suppressed.reset(event.button);
                info.cancelled = true;
                return;
            }
            if (event.state != 0 && ((owned() && blocked && protectedPoint(point)) ||
                (ownerAlive() && !railInputReady() && railPoint(point) && !validPocketDrag()) || pendingPanelPoint(point))) {
                suppressed.set(event.button);
                info.cancelled = true;
            }
        });
    pinMotionListener = g_layoutManager->dragController()->m_events.motion.listen([] { samplePinDrag(); });
    pinEndedListener = g_layoutManager->dragController()->m_events.ended.listen([] {
        if (pinDrag.window) {
            pinDrag.point = g_pInputManager->getMouseCoordsInternal();
            pinDrag.released = Time::steadyNow();
            pinDrag.dropped = true;
        }
    });
    iconMotionListener = Event::bus()->m_events.input.mouse.move.listen([](const Vector2D&, Event::SCallbackInfo&) {
        updateChromeCursor();
        if (iconFeedback.active) {
            damage();
            return;
        }
        // The tab and grip change only with their hover; damage just them.
        static bool tabHovered = false, gripHovered = false;
        const auto tab = detachTab(), grip = resizeGrip();
        const auto point = g_pInputManager->getMouseCoordsInternal();
        const bool onTab = tab.w > 0 && tab.containsPoint(point), onGrip = grip.w > 0 && grip.containsPoint(point);
        if (onTab != tabHovered || onGrip != gripHovered) {
            tabHovered = onTab;
            gripHovered = onGrip;
            for (const auto& box : {tab, grip})
                if (box.w > 0)
                    g_pHyprRenderer->damageBox(box.copy().expand(2));
        }
    });
    axisListener = Event::bus()->m_events.input.mouse.axis.listen(
        [](const IPointer::SAxisEvent &, Event::SCallbackInfo &info) {
            const auto point = g_pInputManager->getMouseCoordsInternal();
            if ((owned() && blocked && protectedPoint(point)) ||
                (ownerAlive() && !railInputReady() && railPoint(point) && !validPocketDrag()) || pendingPanelPoint(point))
                info.cancelled = true;
        });
    frameListener = Event::bus()->m_events.render.preChecks.listen([](PHLMONITOR m) {
        if (m != output)
            return;
        if (!wallpaperCurrent && output->m_activeWorkspace)
            wallpaperCurrent = output->m_activeWorkspace;
        backingPass.reset();
        pocketPass.reset();
        liftedPasses.clear();
        smallPending = false;
        openingArmed = closingArmed = false;
        prune();
        sampleIconFeedback();
        updateWorkAreaViewport();
        updateBarRegions();
        prunePanelContours();
        updateRail();
        updatePanelAnchors();
        refresh();
        alignResizeBacking();
        updateChromeCursor();
        workspaceFullscreenPainted = false;
        tickSearchDim();
        tickOverviewBackdrop();
        tickPocket(Time::steadyNow());
        hideParkingWorkspace();
        const auto focused = Desktop::focusState()->window();
        if ((owned() && blocked && focused == app) || parked(focused) ||
            (rail.blocked && focused == rail.window))
            Desktop::focusState()->fullWindowFocus(nullptr,
                                                   Desktop::FOCUS_REASON_SWITCH_TO_WINDOW_SOFT);
        if (!active || !owned() || foreign || detached)
            return;
        if (restoredContentReady() && app->m_workspace && !app->m_workspace->visible()) {
            clearPictures();
            unblock();
            return;
        }
        if (!restoring && !started && goal == 1 && !failed) {
            if (gate.ready() && backingReady() &&
                app->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT) == wanted) {
                started = true;
                from = progress = resizeReattach ? 1 : pocket.handoff && pocket.box == pane ? pocket.progress : 0;
                pocket.openedFrom = lastOpenFrom = from;
                pocket.handoff = pocket.requested = resizeReattach = false;
                pocketGoal(0);
                began = Time::steadyNow();
                animating = from < 1;
            } else if (Time::steadyNow() >= deadline)
                fail("opening-timeout");
        }
        updateRestoreProgress(Time::steadyNow());
        if (animating) {
            const auto elapsed = std::chrono::duration<double>(Time::steadyNow() - began).count();
            progress = appProgress(elapsed);
            animating = windowMotion && elapsed < appTravelSeconds();
        }
        if (animating || (blocked && !failed)) {
            damage();
            if (restoring && app->m_workspace && !app->m_workspace->visible() &&
                app->backend().surface())
                app->backend().surface()->frame(Time::steadyNow());
        }
    });
    renderListener = Event::bus()->m_events.render.stage.listen([](eRenderStage stage) {
        if (stage == RENDER_BEGIN)
            liftedPasses.clear();
        if (stage == RENDER_POST_TOP_LAYERS && g_pHyprRenderer->m_renderData.pMonitor == output && !redirect) {
            for (auto &lifted : liftedPasses)
                g_pHyprRenderer->addPassElement(std::move(lifted));
            liftedPasses.clear();
        }
        if (stage == RENDER_PRE && railFramesUntil) {
            const auto w = railSource();
            if (!w || Time::steadyNow() >= *railFramesUntil)
                railFramesUntil.reset();
            else {
                wakePanelFrame(w);
                g_pHyprRenderer->damageMonitor(output.lock());
            }
        }
        if (stage == RENDER_PRE && panelFrameWake) {
            const auto w = panelFrameWake->window.lock();
            if (!panelOwned(w) || Time::steadyNow() >= panelFrameWake->until)
                panelFrameWake.reset();
            else {
                wakePanelFrame(w);
                g_pHyprRenderer->damageMonitor(output.lock());
            }
        }
        if (stage == RENDER_POST_WINDOW) {
            const auto window = g_pHyprRenderer->m_renderData.currentWindow;
            if (window && output && window->m_workspace == output->m_activeWorkspace &&
                Fullscreen::controller()->isFullscreen(window.lock()))
                workspaceFullscreenPainted = true;
        }
        paintOverviewBackdrop(stage);
        paintSearchDim(stage);
        if (stage == RENDER_POST_WALLPAPER)
            paintWorkspaceTransition();
        if (stage == RENDER_POST_WINDOWS && g_pHyprRenderer->m_renderData.pMonitor == output && !redirect) {
            if (iconFeedback.active)
                g_pHyprRenderer->addPassElement(makeUnique<IconFeedbackPass>());
            if (detachTab().w > 0)
                g_pHyprRenderer->addPassElement(makeUnique<DetachTabPass>());
        }
        const auto w = g_pHyprRenderer->m_renderData.currentWindow;
        if (w && resizeReattach)
            alignResizeBacking(w.lock());
        const bool liveResize = active && owned() && !foreign && !failed && resizeReattach && (detached || !started);
        const bool bg = w && w->mapped() && w->backend().pid() == ownerPid &&
                        w->backend().metadata().appID == "dev.enzui.SidebarBacking" && w->m_monitor == output &&
                        g_pHyprRenderer->m_renderData.pMonitor == output && !(liveResize && !detached);
        const bool railWindow = w && w->mapped() && w->backend().pid() == ownerPid &&
                                w->backend().metadata().appID == "dev.enzui.Sidebar" && w->m_monitor == output &&
                                g_pHyprRenderer->m_renderData.pMonitor == output;
        const bool controlShape = shapedControl(w.lock()) && g_pHyprRenderer->m_renderData.pMonitor == output;
        const bool target = active && owned() && !foreign && !(restoring && failed) && w == app &&
                            g_pHyprRenderer->m_renderData.pMonitor == output;
        if (stage == RENDER_PRE_WINDOW && target)
            paintResizeBackdrop();
        const bool retained = target && (detached || restoreFrame.has_value()) && !liveResize;
        const bool restored = target && restoredContentReady();
        // The frame's panels and the rail's pane are drawn above the frame,
        // which would otherwise shade them along their join.
        const bool liftedPanel = joinedPanel(w.lock()) && g_pHyprRenderer->m_renderData.pMonitor == output;
        const bool liftedPane = target && !detached && !liveResize && (!restored || retained);
        const bool lifted = liftedPanel || liftedPane;
        const auto place = [lifted](UP<IPassElement> element) {
            if (lifted)
                liftedPasses.push_back(std::move(element));
            else
                g_pHyprRenderer->addPassElement(std::move(element));
        };
        const auto barPicture = (stage == RENDER_PRE_WINDOW && w && !(barFullscreenSuppressed && w == bar) &&
                                 g_pHyprRenderer->m_renderData.pMonitor == output)
                                    ? barPictureBox(w.lock())
                                    : std::nullopt;
        if (barPicture)
            paintBarPicture(w.lock(), *barPicture);
        // The window's own drawing is dropped while its picture is drawn above.
        const bool hidden = parked(w.lock()) || (barFullscreenSuppressed && w == bar) || barPicture.has_value();
        if (pocketPass && g_pHyprRenderer->m_renderData.pMonitor == output && !redirect &&
            ((stage == RENDER_PRE_WINDOW && w && (w == pocketDragWindow() || target)) ||
             stage == RENDER_POST_WINDOWS))
            liftedPasses.push_back(std::move(pocketPass));
        if (stage == RENDER_PRE_WINDOW &&
            (railWindow || controlShape || hidden || bg || retained || lifted || (target && !liveResize && !restored))) {
            if (redirect) {
                fail("nested-pass");
                return;
            }
            capturingBacking = bg;
            discardingParked = hidden;
            captured = makeUnique<Render::CRenderPass>();
            captured->add(
                makeUnique<CRendererHintsPassElement>(CRendererHintsPassElement::SData{}));
            auto *pass = captured.get();
            if (railWindow)
                g_pHyprRenderer->addPassElement(makeUnique<RailPass>(std::move(captured), w.lock()));
            else if (controlShape)
                place(makeUnique<PanelShapePass>(std::move(captured), w.lock()));
            else if (!bg && !hidden) {
                if (retained)
                    place(makeUnique<RetainedWindow>(std::move(captured)));
                else if (target && !liveResize && !restored)
                    place(makeUnique<Group>(std::move(captured)));
                else
                    place(makeUnique<LiftedWindow>(std::move(captured)));
            }
            redirect = g_pHyprRenderer->redirectPass(pass);
        } else if (stage == RENDER_POST_WINDOW && redirect) {
            redirect.reset();
            if (capturingBacking && captured && !captured->single()) {
                if (resizeBackdropActive() && detached) {
                    if (w->backend().surface())
                        w->backend().surface()->presentFeedback(Time::steadyNow(), w->m_monitor.lock(), true);
                } else if (pocketRenderable() && backingReadyFor(pocket.box))
                    pocketPass = makeUnique<PocketPass>(std::move(captured), w.lock());
                else if (active && !restoring && !foreign && !detached)
                    backingPass = std::move(captured);
                else if (w->backend().surface())
                    w->backend().surface()->presentFeedback(Time::steadyNow(), w->m_monitor.lock(),
                                                            true);
            }
            if (discardingParked && w && w->backend().surface())
                w->backend().surface()->presentFeedback(Time::steadyNow(), w->m_monitor.lock(),
                                                        true);
            captured.reset();
        } else if (stage == RENDER_POST_WINDOW && restored)
            g_pHyprRenderer->addPassElement(makeUnique<Marker>());
    });
}
static void exit(HANDLE handle) {
    searchDim = {};
    damage();
    renderListener.reset();
    frameListener.reset();
    workspaceSwitchListener.reset();
    wallpaperCurrent.reset();
    wallpaperPrevious.reset();
    openListener.reset();
    buttonListener.reset();
    axisListener.reset();
    iconMotionListener.reset();
    pinMotionListener.reset();
    pinEndedListener.reset();
    pinDrag = {};
    iconFeedback = {};
    pointerFocusListener.reset();
    keyboardFocusListener.reset();
    redirect.reset();
    captured.reset();
    pocketPass.reset();
    liftedPasses.clear();
    clear();
    clearWorkAreaViewport();
    qualifications.clear();
    suppressed.reset();
    pocket = {};
    clearRail();
    resetBarRegions();
    panelContours.clear();
    nativeTiling = false;
    panelAnchors.clear();
    panelFrameWake.reset();
    railFramesUntil.reset();
    g_pHyprRenderer->currentPass().removeAllOfType("EnzSidebarScene");
    g_pHyprRenderer->currentPass().removeAllOfType("EnzSidebarPocket");
    g_pHyprRenderer->currentPass().removeAllOfType("EnzSidebarRail");
    g_pHyprRenderer->currentPass().removeAllOfType("EnzSidebarDetachTab");
    g_pHyprRenderer->currentPass().removeAllOfType("EnzSidebarIconFeedback");
    g_pHyprRenderer->currentPass().removeAllOfType("EnzControlShape");
    for (const auto *name :
         {"version", "configure", "search_dim", "motion", "motion_state", "supports_islands", "supports_dock", "bar_mode", "bar_regions", "bar_regions_state", "input_at", "panel_frame", "rail", "rail_state", "icon_drop", "cancel_icon_drop", "icon_feedback", "drag_active", "drag_drop", "drag_state", "tiling_mode", "place_tile", "fullscreen_area", "rail_context", "anchor_panel", "panel_state", "pocket", "pocket_state", "hold_pocket", "repin", "supports", "owner_alive", "prepare", "restore", "relocated",
          "show", "hide", "status", "size_limits", "client_resize_disabled", "wallpaper_transition_state", "finish", "counters", "surface_state"})
        HyprlandAPI::removeLuaFunction(handle, "enzui_sidebar_scene", name);
    bar.reset();
    backing.reset();
    parkingWorkspace.reset();
    output.reset();
}
