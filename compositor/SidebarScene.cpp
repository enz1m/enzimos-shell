#include "CommittedGeometry.hpp"
#include "ContentReadiness.hpp"
#include "FloatingChrome.hpp"
#include "IconDropLease.hpp"
#include "RestorePicture.hpp"
#include "SceneTiming.hpp"
#include "src/config/ConfigValue.hpp"
#include "src/render/pass/RectPassElement.hpp"
#include "src/workspace/HLWorkspace.hpp"
#include "src/desktop/state/FocusState.hpp"
#include "src/desktop/state/WindowState.hpp"
#include "src/desktop/view/Popup.hpp"
#include "src/desktop/view/LayerSurface.hpp"
#include "src/desktop/view/window/Window.hpp"
#include "src/desktop/view/window/WindowEffectsController.hpp"
#include "src/layout/LayoutManager.hpp"
#include "src/layout/space/Space.hpp"
#include "src/layout/algorithm/Algorithm.hpp"
#include "src/layout/algorithm/TiledAlgorithm.hpp"
#include "src/managers/SeatManager.hpp"
#include "src/managers/fullscreen/FullscreenController.hpp"
#include "src/managers/input/InputManager.hpp"
#include "src/output/Monitor.hpp"
#include "src/plugins/PluginAPI.hpp"
#include "src/pointer/cursor/CursorShapeOverrideController.hpp"
#include "src/protocols/XDGShell.hpp"
#include "src/protocols/LayerShell.hpp"
#include "src/protocols/core/Compositor.hpp"
#include "src/render/Renderer.hpp"
#include "src/config/shared/complex/ComplexDataTypes.hpp"
#include "src/render/pass/RendererHintsPassElement.hpp"
#include <algorithm>
#include <array>
#include <bitset>
#include <chrono>
#include <cmath>
#include <dlfcn.h>
#include <map>
#include <unordered_set>

namespace EnzMaterial {
// Whether window `w` shows compositor glass (Material.cpp, built into the
// same plugin).
bool hasGlass(PHLWINDOW w);
}

namespace SidebarScene {
using RendererSlot = decltype(g_pHyprRenderer);
using SeatSlot = decltype(g_pSeatManager);
using InputSlot = decltype(g_pInputManager);
using LayoutSlot = decltype(g_layoutManager);
using CursorSlot = decltype(&Pointer::Cursor::overrideController);
static RendererSlot *rendererSlot;
static SeatSlot *seatSlot;
static InputSlot *inputSlot;
static LayoutSlot *layoutSlot;
static CursorSlot cursorSlot;
} // namespace SidebarScene
#define g_pHyprRenderer (*SidebarScene::rendererSlot)
#define g_pSeatManager (*SidebarScene::seatSlot)
#define g_pInputManager (*SidebarScene::inputSlot)
#define g_layoutManager (*SidebarScene::layoutSlot)

namespace SidebarScene {
static constexpr auto inputReason = static_cast<Desktop::View::eFocusBlockReason>(128);
static_assert(static_cast<unsigned>(Desktop::View::FOCUS_BLOCK_GROUP_INACTIVE) == 1 &&
              static_cast<unsigned>(Desktop::View::FOCUS_BLOCK_MONOCLE_INACTIVE) == 2 &&
              static_cast<unsigned>(Desktop::View::FOCUS_BLOCK_BELOW_FULLSCREEN) == 4);
static PHLWINDOWREF app, bar, backing;
static std::unordered_set<uint64_t> existingPeers;
static PHLMONITORREF output;
static PHLWORKSPACEREF parkingWorkspace;
static pid_t ownerPid = 0;
static CBox wanted, pane;
static std::optional<Vector2D> iconAnchor;
static bool active = false, restoring = false, failed = false, foreign = false;
static bool relocationConfirmed = false;
static bool detached = false, resizeReattach = false, desktopRequired = false;
static bool blocked = false, openingArmed = false, closingArmed = false, closed = false;
static bool animating = false, started = false, right = false;
static double progress = 0, from = 0, goal = 1, lastOpenFrom = -1;
static int motionRate = 100;
static bool windowMotion = true;
static const char *failureReason = "none";
static Time::steady_tp began, deadline;
static bool resizeBackdropHeld = false;
static Enzui::ContentReadiness gate;
static uint64_t ticket = 0, rendered = 0, reused = 0, dismissed = 0;
static std::bitset<768> suppressed;
static CHyprSignalListener renderListener, frameListener, commitListener, contentListener;
static CHyprSignalListener buttonListener, axisListener, openListener;
static CHyprSignalListener pointerFocusListener, keyboardFocusListener, workspaceSwitchListener, monitorRemovedListener;
static UP<Render::CRenderPass> captured, backingPass;
static UP<IPassElement> pocketPass;
// Drawn after the top layers: windows that join the display frame, so its
// shadow falls beneath them, not across them.
static std::vector<UP<IPassElement>> liftedPasses;
static UP<Render::CScopeGuard> redirect;
static bool capturingBacking = false, discardingParked = false;
// Fullscreen workspaces draw floating overlays both below and above their fullscreen client.
static bool workspaceFullscreenPainted = false;
struct SmallFrame {
    SP<Render::IFramebuffer> image;
    PHLWINDOWREF window;
    CBox box;
    uint64_t epoch = 0;
};
struct RestoreFrame {
    SmallFrame source;
    SP<Render::IFramebuffer> destination;
    CBox visible;
    double progress = 0, from = 0;
    bool started = false;
    Time::steady_tp began;
};
static SmallFrame smallCommitted;
static SP<Render::IFramebuffer> smallScratch;
static bool smallPending = false;
static CBox smallPendingBox;
static std::optional<RestoreFrame> restoreFrame;
struct Pocket {
    CBox box;
    bool right = false, requested = false, hover = false, handoff = false;
    double progress = 0, from = 0, goal = 0;
    double openedFrom = -1;
    Time::steady_tp began, lease;
};
static Pocket pocket;
struct Rail {
    CBox box;
    PHLWINDOWREF window;
    double solid = 0, frame = 0, progress = 0, from = 0, goal = 0;
    bool right = false, configured = false, blocked = false, floating = false, clientShape = false;
    WP<CWLSurfaceResource> maskSurface;
    CRegion originalMask, mask;
    Vector2D maskSize;
    bool originalInfinite = true;
    Time::steady_tp began;
};
static Rail rail;
static double appTravelSeconds() {
    return iconAnchor && rail.floating ? Enzui::iconTravelSeconds(from, goal, motionRate)
                                      : Enzui::sceneTravelSeconds(from, goal, motionRate);
}
static double appProgress(double seconds) {
    if (!windowMotion)
        return goal;
    return iconAnchor && rail.floating ? Enzui::iconProgress(from, goal, seconds, motionRate)
                                      : Enzui::sceneProgress(from, goal, seconds, motionRate);
}
static Enzui::IconDropLease iconDrop;
static PHLWINDOWREF iconDropSource;
static PHLMONITORREF iconDropOutput;
static pid_t iconDropOwner = 0;
struct PanelAnchor {
    PHLWINDOWREF window;
    CBox target, guard;
    double initialX = 0;
    bool bottom = true;
    bool pending = true;
    size_t moves = 0;
};
static std::vector<PanelAnchor> panelAnchors;
struct PanelFrameWake { PHLWINDOWREF window; Time::steady_tp until; };
static std::optional<PanelFrameWake> panelFrameWake;
// The rail is drawn from its window's picture, and the scene sends that
// window frame callbacks only when it draws: while the shell changes the
// rail's look (a style switch, a new colour) they keep coming until then.
static std::optional<Time::steady_tp> railFramesUntil;

static bool remote(PHLWINDOW w) {
    if (!w || !w->m_monitor || w->m_monitor->m_name.starts_with("ENZREMOTE-"))
        return true;
    for (const auto &tag : w->m_ruleApplicator->m_tagKeeper.getTags())
        if (tag.starts_with("enzremote-"))
            return true;
    return false;
}
static bool owned() {
    return app && app->mapped() && output && output->enabled() && app->m_monitor == output &&
           !remote(app.lock());
}
static bool ownerAlive() {
    return bar && bar->mapped() && bar->backend().pid() == ownerPid && bar->m_monitor == output &&
           bar->backend().metadata().appID == "dev.enzui.Desktop";
}
static bool fullscreenOnOutput() {
    if (!output || !ownerAlive()) return false;
    for (const auto& w : Desktop::windowState()->windows()) {
        if (w->mapped() && !w->isHidden() && w->m_monitor == output &&
            w->m_workspace == output->m_activeWorkspace && w->backend().pid() != ownerPid && !remote(w) &&
            Fullscreen::controller()->getFullscreenModes(w).internal == Fullscreen::FSMODE_FULLSCREEN)
            return true;
    }
    return false;
}
static bool railSuppressed() { return rail.goal == 0 && fullscreenOnOutput(); }
static bool supported(PHLWINDOW w) {
    return w && w->mapped() && !w->isHidden() && w->backend().pid() != ownerPid &&
           !w->backend().isX11() && !w->backend().parent() && !w->backend().traits().modal &&
           !w->backend().traits().transient && !w->effects().hasActiveTransformers() && !remote(w);
}
static bool additionalWindow();
static bool draggingApp() {
    const auto &drag = g_layoutManager->dragController();
    const auto target = drag->target();
    return target && target->type() == Layout::TARGET_TYPE_WINDOW && target->window() == app &&
           (drag->mode() == MBIND_MOVE || drag->mode() == MBIND_RESIZE ||
            drag->mode() == MBIND_RESIZE_FORCE_RATIO || drag->mode() == MBIND_RESIZE_BLOCK_RATIO);
}
static bool resizingApp() {
    const auto &drag = g_layoutManager->dragController();
    return draggingApp() && drag->mode() != MBIND_MOVE;
}
static bool chromeHeld = false, resizeHover = false;
static std::string chromeCursor;
static CBox detachTab() {
    if (!owned() || foreign || restoring || detached || failed || blocked || !started || progress != 1 || additionalWindow())
        return {};
    const auto box = app->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT);
    if (box != wanted || box.w < 80 || box.h < 80)
        return {};
    // On the pane's outer edge, away from the rail: the side it is pulled
    // out from. It sat on the right edge for a right-hand sidebar too.
    return {right ? box.x : box.x + box.w - 16, box.y + (box.h - 72) / 2, 16, 72};
}
static CBox resizeGrip() {
    if (detachTab().w <= 0 || wanted.h < 160)
        return {};
    const auto box = app->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT);
    // In the pane's outer bottom corner, as the policy's press test has it.
    return {right ? box.x + 6 : box.x + box.w - 28, box.y + box.h - 28, 22, 22};
}
static void updateChromeCursor() {
    if (!cursorSlot || !*cursorSlot)
        return;
    const auto point = g_pInputManager->getMouseCoordsInternal();
    const bool available = !chromeHeld && !g_layoutManager->dragController()->target();
    const bool resizing = available && resizeGrip().containsPoint(point);
    const bool dragging = available && detachTab().containsPoint(point);
    std::string shape;
    if (resizing)
        shape = right ? "sw-resize" : "se-resize";
    else if (dragging)
        shape = "grab";
    else if (available && owned() && !detached) {
        const auto box = app->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT);
        if (box.containsPoint(point) &&
            (point.x < box.x + 8 || point.x >= box.x + box.w - 8 ||
             point.y < box.y + 8 || point.y >= box.y + box.h - 8))
            shape = "default";
    }
    if (shape == chromeCursor && resizing == resizeHover)
        return;
    chromeCursor = shape;
    resizeHover = resizing;
    if (shape.empty())
        (*cursorSlot)->unsetOverride(Pointer::Cursor::CURSOR_OVERRIDE_UNKNOWN);
    else
        (*cursorSlot)->setOverride(shape, Pointer::Cursor::CURSOR_OVERRIDE_UNKNOWN);
}
static bool hasConflictingWindow(PHLWINDOW target, bool watchNewPeers) {
    if (!target)
        return false;
    for (const auto &other : Desktop::windowState()->windows()) {
        if (!other->mapped() || other == target)
            continue;
        auto parent = other->backend().parent();
        if (other->backend().pid() == target->backend().pid()) {
            // A client may open an unparented dialog after its sidebar window is shown.
            if ((watchNewPeers && !existingPeers.contains(other->metadata().stableID())) ||
                (!parent && other->backend().traits().modal))
                return true;
        }
        for (unsigned depth = 0; parent && depth < 16; ++depth) {
            if (parent == target)
                return true;
            parent = parent->backend().parent();
        }
    }
    return false;
}
static bool additionalWindow() { return hasConflictingWindow(app.lock(), true); }
static std::string address(PHLWINDOW w) { return std::format("0x{:x}", uintptr_t(w.get())); }
static PHLWINDOW find(lua_State *L) {
    if (!lua_isstring(L, 1))
        return {};
    for (const auto &w : Desktop::windowState()->windows())
        if (address(w) == lua_tostring(L, 1))
            return w;
    return {};
}
static bool selected(lua_State *L) {
    return app && lua_isstring(L, 1) && address(app.lock()) == lua_tostring(L, 1);
}
static void damage() {
    if (output)
        g_pHyprRenderer->damageMonitor(output.lock());
}
static PHLWINDOW railSource() {
    PHLWINDOW found;
    for (const auto &w : Desktop::windowState()->windows()) {
        if (w->mapped() && w->backend().pid() == ownerPid && w->m_monitor == output &&
            w->backend().metadata().appID == "dev.enzui.Sidebar") {
            if (found)
                return nullptr;
            found = w;
        }
    }
    return found;
}
static bool railReady() {
    if (!rail.configured || !ownerAlive() || !output->enabled())
        return false;
    const auto w = railSource();
    const auto surface = w ? w->backend().surface() : nullptr;
    return surface && surface->m_current.texture &&
           w->backend().geometry().box.size() == rail.box.size() &&
           surface->m_current.size == rail.box.size() &&
           w->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT) ==
               rail.box.copy().translate(output->m_position);
}
static Vector2D railShift() {
    if (!rail.configured)
        return {};
    if (!rail.floating || !output)
        return {(rail.right ? 1 : -1) * Enzui::railOffset(rail.solid, rail.frame, rail.progress), 0.0};
    const double travel = rail.right ? output->m_size.x - rail.box.x : rail.box.x + rail.box.w;
    return {(rail.right ? 1 : -1) * travel * (1.0 - rail.progress), 0.0};
}
static bool railInputReady() {
    if (!railReady() || rail.goal != 1 || rail.progress != 1)
        return false;
    const auto source = railSource();
    return !rail.clientShape || (source && !source->backend().surface()->m_current.inputIsInfinite &&
                                 !source->backend().surface()->m_current.input.empty());
}
static bool railPoint(const Vector2D &point) {
    if (!rail.configured || !output || railSuppressed())
        return false;
    auto box = rail.box.copy();
    if (rail.floating) {
        const auto local = point - output->m_position;
        const auto shifted = box.copy().translate(railShift());
        if (rail.clientShape) {
            const auto source = railSource();
            const auto surface = source ? source->backend().surface() : nullptr;
            return surface && !surface->m_current.inputIsInfinite && CBox{0, 0, output->m_size.x, output->m_size.y}.containsPoint(local) &&
                   surface->m_current.input.containsPoint(local - shifted.pos());
        }
        return CBox{0, 0, output->m_size.x, output->m_size.y}.containsPoint(local) && Enzui::capsuleContains(shifted.w, shifted.h, local.x - shifted.x, local.y - shifted.y);
    }
    if (rail.right)
        box.x += box.w - rail.solid;
    box.w = rail.solid;
    return box.translate(output->m_position).containsPoint(point);
}
static void sampleRail(Time::steady_tp now) {
    if (!rail.configured)
        return;
    rail.progress = Enzui::sceneProgress(rail.from, rail.goal,
        std::chrono::duration<double>(now - rail.began).count(), motionRate);
}
static void restoreRailMask() {
    if (rail.maskSurface && rail.window && rail.window->mapped() &&
        rail.window->backend().surface() == rail.maskSurface.lock()) {
        auto& state = rail.maskSurface->m_current;
        if (!state.inputIsInfinite && pixman_region32_equal(state.input.pixman(), rail.mask.pixman())) {
            state.input = rail.originalMask;
            state.inputIsInfinite = rail.originalInfinite;
        }
    }
    rail.maskSurface.reset();
    rail.maskSize = {};
    rail.mask.clear();
}
static bool updateRailMask() {
    if (!rail.floating || rail.clientShape) {
        const bool changed = !rail.maskSurface.expired();
        restoreRailMask();
        return changed;
    }
    if (!railReady())
        return false;
    const auto surface = rail.window->backend().surface();
    if (surface != rail.maskSurface.lock()) {
        restoreRailMask();
        rail.maskSurface = surface;
        rail.originalMask = surface->m_current.input;
        rail.originalInfinite = surface->m_current.inputIsInfinite;
    } else if (surface->m_current.inputIsInfinite ||
               !pixman_region32_equal(surface->m_current.input.pixman(), rail.mask.pixman())) {
        rail.originalMask = surface->m_current.input;
        rail.originalInfinite = surface->m_current.inputIsInfinite;
    }
    if (rail.maskSize != rail.box.size()) {
        rail.mask.clear();
        for (const auto& span : Enzui::capsuleInput(static_cast<int>(rail.box.w), static_cast<int>(rail.box.h)))
            rail.mask.add(span.x, span.y, span.width, span.height);
        rail.maskSize = rail.box.size();
    }
    auto& state = surface->m_current;
    if (!state.inputIsInfinite && pixman_region32_equal(state.input.pixman(), rail.mask.pixman()))
        return false;
    state.input = rail.mask;
    state.inputIsInfinite = false;
    return true;
}
static void clearRail() {
    restoreRailMask();
    if (rail.window && rail.blocked)
        rail.window->setInputBlocked(inputReason, false);
    rail = {};
}
static bool panelOwned(PHLWINDOW w) {
    if (!ownerAlive() || !w || !w->mapped() || w->backend().pid() != ownerPid ||
        w->m_monitor != output || remote(w))
        return false;
    const auto &name = w->backend().metadata().appID;
    return name == "dev.enzui.Desktop" || name == "dev.enzui.ControlCenter" || name == "dev.enzui.PluginPanel" ||
           name == "dev.enzui.NotificationCenter" ||
           (name == "dev.enzui.QuickSearch" && w->m_workspace &&
            w->m_workspace->type() == Workspace::eWorkspaceType::NORMAL);
}
// The shell's panels that open out of the bar or the frame, and the
// assistant's tab: part of the frame, drawn above it.
static bool joinedPanel(PHLWINDOW w) {
    if (!ownerAlive() || !w || !w->mapped() || w->backend().pid() != ownerPid || w->m_monitor != output || remote(w))
        return false;
    const auto &name = w->backend().metadata().appID;
    return name == "dev.enzui.ControlCenter" || name == "dev.enzui.PluginPanel" || name == "dev.enzui.NotificationCenter" ||
           name == "dev.enzui.WorkspaceOverview" || name == "dev.enzui.AssistantPanel" ||
           name == "dev.enzui.AssistantTrigger" || name == "dev.enzui.QuickSearch";
}
static bool pendingPanelPoint(const Vector2D &point) {
    return std::ranges::any_of(panelAnchors, [&](const auto &anchor) {
        return anchor.pending && panelOwned(anchor.window.lock()) &&
               (anchor.guard.containsPoint(point) || anchor.target.containsPoint(point));
    });
}
static void updatePanelAnchors() {
    std::erase_if(panelAnchors, [](const auto &anchor) { return !panelOwned(anchor.window.lock()); });
    for (auto &anchor : panelAnchors) {
        const auto w = anchor.window.lock();
        const auto surface = w->backend().surface();
        if (!surface || !surface->m_current.texture || surface->m_current.size.y < 1)
            continue;
        const auto size = surface->m_current.size;
        anchor.pending = size != anchor.target.size() || w->backend().geometry().box.size() != anchor.target.size();
        const auto painted =
            Enzui::paintedHeight(size.y, w->size(Desktop::View::IGeometric::GEOMETRIC_CURRENT).y);
        const Vector2D position{anchor.pending ? anchor.initialX : anchor.target.x,
                                Enzui::anchoredY(anchor.target.y, anchor.target.h, painted, anchor.bottom)};
        anchor.guard = CBox{position, size};
        if (w->position(Desktop::View::IGeometric::GEOMETRIC_CURRENT) == position)
            continue;
        const auto command = std::format("hl.dispatch(hl.dsp.window.move({{window=\"address:{}\",x={},y={}}}))",
                                         address(w), position.x, position.y);
        if (HyprlandAPI::invokeHyprctlCommand("eval", command) == "ok")
            ++anchor.moves;
        damage();
    }
}
static void samplePocket(Time::steady_tp now) {
    const auto seconds = std::chrono::duration<double>(now - pocket.began).count();
    pocket.progress = Enzui::pocketProgress(pocket.from, pocket.goal, seconds, motionRate);
}
static void pocketGoal(double goal) {
    if (pocket.goal == goal)
        return;
    samplePocket(Time::steadyNow());
    pocket.from = pocket.progress;
    pocket.goal = goal;
    pocket.began = Time::steadyNow();
    damage();
}
static void holdPocket() {
    samplePocket(Time::steadyNow());
    pocket.from = pocket.goal = pocket.progress;
    pocket.began = Time::steadyNow();
    pocket.handoff = true;
    pocket.lease = pocket.began + std::chrono::seconds(1);
}
static PHLWINDOW pocketDragWindow() {
    const auto &drag = g_layoutManager->dragController();
    const auto target = drag->target();
    if (target && drag->mode() == MBIND_MOVE && target->type() == Layout::TARGET_TYPE_WINDOW &&
        supported(target->window()) && target->window()->m_monitor == output)
        return target->window();
    return nullptr;
}
static bool validPocketDrag() { return !!pocketDragWindow(); }
static bool pocketRenderable() {
    return !rail.floating && pocket.progress > 0 && ownerAlive() && railReady() &&
           (!active || detached || restoring || !started || failed || closed);
}
static void tickPocket(Time::steady_tp now) {
    if (!ownerAlive() || rail.floating) {
        if (pocket.progress > 0)
            damage();
        pocket = {};
        return;
    }
    if (failed || foreign || now > pocket.lease || (pocket.handoff && (!active || !owned())) ||
        (pocket.requested && !pocket.handoff && !validPocketDrag())) {
        pocket.handoff = pocket.requested = false;
        pocketGoal(0);
    }
    samplePocket(now);
    if (pocket.progress != pocket.goal)
        damage();
}
static void clearPictures() {
    smallCommitted = {};
    smallScratch.reset();
    smallPending = false;
    restoreFrame.reset();
}
static bool protectedPoint(const Vector2D &point) {
    return wanted.copy().expand(Enzui::protectedContentPadding(rail.floating, restoring)).containsPoint(point) ||
           (restoreFrame && restoreFrame->visible.copy().expand(6).containsPoint(point));
}
static void surfaces(PHLWINDOW w, const std::function<void(SP<CWLSurfaceResource>)> &visit) {
    const auto tree = [&](SP<CWLSurfaceResource> s) {
        if (s)
            s->breadthfirst(
                [&](SP<CWLSurfaceResource> child, const Vector2D &, void *) { visit(child); },
                nullptr);
    };
    tree(w->backend().surface());
    if (w->popupHead())
        w->popupHead()->breadthfirst(
            [&](SP<Desktop::View::CPopup> popup, void *) {
                if (popup->wlSurface())
                    tree(popup->wlSurface()->resource());
            },
            nullptr);
}
static std::vector<PHLWINDOW> parkedRoots();
static void clearBlockedFocus() {
    auto roots = parkedRoots();
    if (owned() && blocked)
        roots.push_back(app.lock());
    if (rail.window && rail.blocked)
        roots.push_back(rail.window.lock());
    bool pointer = false, keyboard = false;
    for (const auto &root : roots)
        surfaces(root, [&](SP<CWLSurfaceResource> s) {
            pointer |= g_pSeatManager->m_state.pointerFocus == s;
            keyboard |= g_pSeatManager->m_state.keyboardFocus == s;
        });
    if (pointer)
        g_pSeatManager->setPointerFocus(nullptr, {});
    if (keyboard)
        g_pSeatManager->setKeyboardFocus(nullptr);
}
static void updateRail() {
    const auto w = railSource();
    if (rail.window != w) {
        restoreRailMask();
        if (rail.window && rail.blocked)
            rail.window->setInputBlocked(inputReason, false);
        rail.window = w;
        rail.blocked = false;
    }
    sampleRail(Time::steadyNow());
    if (w) {
        const bool maskChanged = updateRailMask();
        const bool blocked = !railInputReady();
        if (blocked != rail.blocked) {
            rail.blocked = blocked;
            w->setInputBlocked(inputReason, blocked);
            if (blocked)
                clearBlockedFocus();
            else
                g_pInputManager->simulateMouseMovement();
        } else if (maskChanged && !blocked)
            g_pInputManager->simulateMouseMovement();
    }
    if (rail.progress != rail.goal)
        damage();
}
static void dismissPopups(PHLWINDOW root) {
    if (!root->popupHead())
        return;
    std::vector<SP<CXDGPopupResource>> closing;
    root->popupHead()->breadthfirst(
        [&](SP<Desktop::View::CPopup> popup, void *) {
            if (!popup->mapped() || !popup->wlSurface() || closing.size() >= 64)
                return;
            auto s = popup->wlSurface()->resource();
            if (!s || !s->m_role || s->m_role->role() != SURFACE_ROLE_XDG_SHELL)
                return;
            auto role = dynamicPointerCast<CXDGSurfaceRole>(s->m_role);
            auto xdg = role ? role->m_xdgSurface.lock() : nullptr;
            if (xdg && xdg->m_popup)
                closing.push_back(xdg->m_popup.lock());
        },
        nullptr);
    for (auto i = closing.rbegin(); i != closing.rend(); ++i) {
        (*i)->done();
        ++dismissed;
    }
    clearBlockedFocus();
}
static void quiescePopups() {
    for (const auto &root : parkedRoots())
        dismissPopups(root);
    if (owned() && blocked)
        dismissPopups(app.lock());
}
static void block() {
    if (app && !blocked) {
        blocked = true;
        app->setInputBlocked(inputReason, true);
    }
    openingArmed = false;
    clearBlockedFocus();
    quiescePopups();
}
static void unblock() {
    if (app && blocked)
        app->setInputBlocked(inputReason, false);
    blocked = false;
}
static void fail(const char *reason) {
    if (failed)
        return;
    failureReason = reason;
    failed = true;
    block();
    progress = from = 0;
    animating = false;
    damage();
}

static Enzui::ContentGeometry geometryOf(PHLWINDOW w) {
    const auto g = w->backend().geometry().box;
    const auto ack = w->backend().reportedSize();
    const auto s = w->backend().surface();
    const auto extent = s ? s->m_current.size : Vector2D{};
    return {g.x, g.y, g.w, g.h, ack.x, ack.y, extent.x, extent.y};
}
struct Qualification {
    PHLWINDOWREF window;
    WP<CWLSurfaceResource> surface;
    WP<Render::ITexture> texture;
    Enzui::CommittedGeometry geometry;
    CHyprSignalListener listener;
    void observe() {
        auto w = window.lock();
        auto s = w ? w->backend().surface() : nullptr;
        if (!s || !s->m_current.updated.bits.buffer)
            return;
        surface = s;
        texture = s->m_current.texture;
        geometry.observe(geometryOf(w), true);
    }
    bool matches(PHLWINDOW w, const Vector2D &size) const {
        auto s = w->backend().surface();
        return window == w && s && surface == s && texture && texture == s->m_current.texture &&
               geometry.matches(geometryOf(w), size.x, size.y);
    }
};
static std::map<uintptr_t, Qualification> qualifications;
static bool parked(PHLWINDOW w) {
    if (!output || !output->enabled() || !w || !w->mapped() || w->backend().pid() == ownerPid ||
        w->m_monitor != output || remote(w) || !w->m_workspace ||
        w->m_workspace->addressableName() != "special:enzui-miniapps")
        return false;
    const auto q = qualifications.find(uintptr_t(w.get()));
    return q != qualifications.end() && q->second.window == w &&
           w->m_ruleApplicator->m_tagKeeper.getTags().contains("enzui-sidebar-app");
}
static std::vector<PHLWINDOW> parkedRoots() {
    std::vector<PHLWINDOW> roots;
    for (const auto &[address, qualification] : qualifications) {
        auto w = qualification.window.lock();
        if (parked(w)) {
            parkingWorkspace = w->m_workspace;
            roots.push_back(w);
        }
    }
    return roots;
}
static void hideParkingWorkspace() {
    if (!output || !parkingWorkspace || output->m_activeSpecialWorkspace != parkingWorkspace)
        return;
    for (const auto &w : Desktop::windowState()->windows())
        if (w->mapped() && w->m_workspace == parkingWorkspace && !parked(w))
            return;
    output->setSpecialWorkspace(nullptr);
}
static void prune() {
    for (auto i = qualifications.begin(); i != qualifications.end();) {
        if (!i->second.window || !i->second.window->mapped())
            i = qualifications.erase(i);
        else
            ++i;
    }
}
static void primeIslandInput(PHLWINDOW w);
static void watch(PHLWINDOW w) {
    primeIslandInput(w);
    prune();
    if (!w || w->backend().isX11() || qualifications.size() >= 64)
        return;
    auto [i, inserted] = qualifications.try_emplace(uintptr_t(w.get()));
    if (!inserted)
        return;
    auto &q = i->second;
    q.window = w;
    q.observe();
    q.listener = w->backend().m_events.commit.listen([ptr = &q](bool) { ptr->observe(); });
}
static bool targetSize(const Vector2D &size) {
    return size == wanted.size() || (restoring && std::abs(size.x - wanted.w) <= 1 && std::abs(size.y - wanted.h) <= 1);
}
static bool targetBox(const CBox &box) {
    return box.x == wanted.x && box.y == wanted.y && targetSize(box.size());
}
static Enzui::ContentReadiness::Commit content() {
    auto s = app ? app->backend().surface() : nullptr;
    if (!s)
        return {};
    const auto g = app->backend().geometry().box;
    const auto &state = s->m_current;
    return {targetSize(app->backend().reportedSize()), targetSize(g.size()),
            g.x >= 0 && g.y >= 0 && g.x + g.w <= state.size.x && g.y + g.h <= state.size.y,
            bool(state.texture), bool(state.updated.bits.buffer && state.buffer)};
}
static bool backingReadyFor(const CBox &box) {
    if (!ownerAlive())
        return false;
    PHLWINDOW found;
    for (auto &w : Desktop::windowState()->windows()) {
        if (w->mapped() && w->backend().pid() == ownerPid && w->m_monitor == output &&
            w->backend().metadata().appID == "dev.enzui.SidebarBacking") {
            if (found)
                return false;
            found = w;
        }
    }
    backing = found;
    if (!found)
        return false;
    auto s = found->backend().surface();
    const auto target = box.copy().translate(output->m_position);
    if (active && !restoring && !detached && box == pane &&
        found->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT).pos() != target.pos()) {
        found->move(target.pos());
        found->finishAnimation();
    }
    return s && s->m_current.texture && found->backend().geometry().box.size() == box.size() &&
           s->m_current.size == box.size() &&
           found->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT) == target;
}
static bool backingReady() { return backingReadyFor(pane); }
// The bar while its box and its last picture disagree, between a size change
// and the bar's next frame: the picture is drawn at its own size from the
// bar's edge, rather than stretched by repeating its last row of pixels into
// a taller box, or cut short in a smaller one.
static std::optional<CBox> barPictureBox(PHLWINDOW w) {
    if (!w || w != bar.lock() || !ownerAlive() || !w->m_monitor || w->m_monitor != output)
        return std::nullopt;
    const auto surface = w->backend().surface();
    if (!surface || !surface->m_current.texture || surface->m_current.size.x < 1 || surface->m_current.size.y < 1)
        return std::nullopt;
    const auto box = w->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT);
    const auto size = surface->m_current.size;
    const auto same = [&](double width, double height) {
        return std::abs(size.x - width) < 0.5 && std::abs(size.y - height) < 0.5;
    };
    // The size it is going to also counts: the box can be placed for the new
    // size a frame before it takes it.
    const auto anchor = std::ranges::find_if(panelAnchors, [&](const auto &a) { return a.window.lock() == w; });
    // And the place: at the bottom the box takes its new size a frame before
    // it is moved to its new place.
    const bool placed = anchor == panelAnchors.end() ||
        (same(anchor->target.w, anchor->target.h) && std::abs(box.x - anchor->target.x) < 0.5 &&
         std::abs(box.y - anchor->target.y) < 0.5);
    const bool settled = same(box.w, box.h) && placed;
    if (settled)
        return std::nullopt;
    // At the edge the bar belongs to, which its anchor names: its box moves as
    // well as grows during the change at the bottom, so the box's own edge
    // wandered for a frame. Without an anchor, the box's nearer edge.
    if (anchor != panelAnchors.end()) {
        const double y = anchor->bottom ? anchor->target.y + anchor->target.h - size.y : anchor->target.y;
        return CBox{anchor->target.x, y, size.x, size.y}.translate(-output->m_position);
    }
    const bool bottom = box.y + box.h / 2 > output->m_position.y + output->m_size.y / 2;
    const double y = bottom ? box.y + box.h - size.y : box.y;
    return CBox{box.x, y, size.x, size.y}.translate(-output->m_position);
}
static void paintBarPicture(PHLWINDOW w, const CBox &local) {
    g_pHyprRenderer->addPassElement(makeUnique<CTexPassElement>(CTexPassElement::SRenderData{
        .tex = w->backend().surface()->m_current.texture, .box = local.copy().scale(output->m_scale).round()}));
}
static bool resizeBackdropActive() {
    return active && owned() && !restoring && !failed && goal == 1 &&
           (resizeReattach || (!detached && started && resizeBackdropHeld));
}
static void paintResizeBackdrop() {
    if (!resizeBackdropActive() || !backing || !backing->mapped())
        return;
    auto surface = backing->backend().surface();
    if (!surface || !surface->m_current.texture)
        return;
    auto box = pane;
    if (detached) {
        const auto inset = (pane.w - wanted.w) / 2;
        box = app->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT).copy()
                  .expand(inset).translate(-output->m_position);
    }
    g_pHyprRenderer->addPassElement(makeUnique<CTexPassElement>(
        CTexPassElement::SRenderData{.tex = surface->m_current.texture, .box = box.copy().scale(output->m_scale).round()}));
}
static void alignResizeBacking(PHLWINDOW window = nullptr) {
    if (!resizeReattach || !owned() || !ownerAlive() || failed)
        return;
    Vector2D target = pane.pos() + output->m_position;
    if (detached) {
        const auto inset = (pane.w - wanted.w) / 2;
        target = app->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT).pos() - Vector2D{inset, inset};
    }
    auto align = [&](PHLWINDOW w) {
        if (w && w->mapped() && w->backend().pid() == ownerPid && w->m_monitor == output &&
            w->backend().metadata().appID == "dev.enzui.SidebarBacking" &&
            w->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT).pos() != target) {
            w->move(target);
            w->positionAnimation()->warp();
        }
    };
    if (window)
        align(window);
    else
        for (auto &w : Desktop::windowState()->windows())
            align(w);
}
static bool restoreOnDesktop() {
    return owned() && restoring && app->m_workspace &&
           app->m_workspace->type() == Workspace::eWorkspaceType::NORMAL &&
           !bool(app->m_state & Desktop::View::WINDOW_STATE_PINNED);
}
static void refresh() {
    quiescePopups();
    if (!active)
        return;
    if (app && app->mapped() && !owned()) {
        foreign = true;
        clearPictures();
        unblock();
        gate.cancel();
        openingArmed = closingArmed = false;
        damage();
        return;
    }
    if (!owned() || foreign)
        return;
    if (!restoring && !detached && !failed && started && progress == 1 && !blocked &&
        draggingApp() &&
        app->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT) != wanted &&
        (resizingApp() || app->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT).size() == wanted.size())) {
        detached = true;
        resizeReattach = resizingApp();
        if (resizeReattach)
            resizeBackdropHeld = true;
        pocket.box = pane;
        pocket.right = right;
        pocket.handoff = false;
        pocket.requested = true;
        pocket.hover = false;
        pocket.progress = pocket.from = progress;
        pocket.goal = 1.0 / 3.0;
        pocket.began = Time::steadyNow();
        pocket.lease = pocket.began + std::chrono::seconds(1);
        gate.cancel();
        animating = openingArmed = closingArmed = false;
        backingPass.reset();
        damage();
    }
    if (detached)
        return;
    if (!restoring && additionalWindow()) {
        desktopRequired = true;
        fail("additional-window");
    }
    const auto now = Time::steadyNow();
    gate.advance(now);
    if (restoring && !relocationConfirmed && now >= deadline)
        fail("relocation-timeout");
    if (restoring && relocationConfirmed &&
        bool(app->m_state & Desktop::View::WINDOW_STATE_PINNED) && now >= deadline)
        fail("unpin-timeout");
    if (!restoring && (!ownerAlive() || !supported(app.lock())))
        fail("unsupported-owner-or-window");
    if (gate.ready() && Enzui::placementState(
            targetBox(app->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT)),
            started && !restoring, now >= deadline) == Enzui::PlacementState::Failed)
        fail("geometry");
    if (!restoring && started && !backingReady())
        fail("backing");
    if (gate.state() == Enzui::ContentReadiness::State::Changed ||
        gate.state() == Enzui::ContentReadiness::State::TimedOut)
        fail("content-changed-or-timeout");
    if (restoring && blocked) {
        if (now >= deadline + std::chrono::seconds(1))
            fail("restore-presentation-timeout");
        if (failed) {
            clearPictures();
            gate.cancel();
            unblock();
            damage();
        }
    }
}
static void clear() {
    chromeHeld = false;
    if (!chromeCursor.empty() && cursorSlot && *cursorSlot)
        (*cursorSlot)->unsetOverride(Pointer::Cursor::CURSOR_OVERRIDE_UNKNOWN);
    chromeCursor.clear();
    resizeHover = false;
    clearPictures();
    unblock();
    damage();
    active = restoring = failed = foreign = animating = started = relocationConfirmed = false;
    openingArmed = closingArmed = closed = detached = resizeReattach = desktopRequired = false;
    lastOpenFrom = -1;
    iconAnchor.reset();
    resizeBackdropHeld = false;
    gate.cancel();
    contentListener.reset();
    commitListener.reset();
    backingPass.reset();
    app.reset();
    existingPeers.clear();
}

static bool restoredContentReady() {
    return restoreOnDesktop() && relocationConfirmed && gate.ready() && !failed &&
           targetBox(app->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT));
}
static bool rasterSupported() {
    return output && output->enabled();
}
// Round origin and size independently to preserve retained texture dimensions at fractional
// scales.
static CBox physical(const CBox &box) {
    return {(box.pos() * output->m_scale).round(), (box.size() * output->m_scale).round()};
}
static bool snapshotPass(Render::CRenderPass &pass, SP<Render::IFramebuffer> &image,
                         const CBox &globalBox) {
    auto &data = g_pHyprRenderer->m_renderData;
    if (!rasterSupported() || pass.single() || pass.needsLiveBlur() ||
        pass.needsPrecomputeBlur() || !data.renderModif.modifs.empty() ||
        globalBox.w < 1 || globalBox.h < 1)
        return false;
    const auto pixels = physical(CBox{{}, globalBox.size()}).size();
    if (pixels.x < 1 || pixels.y < 1 || pixels.x * pixels.y > 8388608)
        return false;
    const auto saved = data;
    const bool blur = output->m_blurFBShouldRender;
    const auto restoreData = makeUnique<Render::CScopeGuard>([saved, blur] {
        g_pHyprRenderer->m_renderData = saved;
        if (output)
            output->m_blurFBShouldRender = blur;
    });
    try {
        if (!image || image->m_size != pixels) {
            image = g_pHyprRenderer->createFB("sidebar retained window");
            if (!image || !image->alloc(pixels.x, pixels.y, DRM_FORMAT_ABGR8888)) {
                image.reset();
                return false;
            }
            image->setImageDescription(output->workBufferImageDescription());
        }
        if (!image->getTexture())
            return false;
        const auto binding = g_pHyprRenderer->bindTempFB(image);
        const CRegion entire{0, 0, static_cast<int>(pixels.x), static_cast<int>(pixels.y)};
        data.surface.reset();
        data.clipBox = {};
        data.damage = entire;
        data.fbSize = pixels;
        data.transformDamage = false;
        data.noSimplify = true;
        data.renderingTransformedSource = true;
        g_pHyprRenderer->setProjectionType(Render::RPT_EXPORT);
        g_pHyprRenderer->draw(CClearPassElement::SClearData{CHyprColor(0, 0, 0, 0)});
        data.renderModif = {};
        data.renderModif.modifs.emplace_back(std::make_pair(
            Render::SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE,
            -physical(globalBox.copy().translate(-output->m_position)).pos()));
        pass.render(entire);
        return true;
    } catch (...) {
        image.reset();
        return false;
    }
}
static void paintPicture(const SP<Render::IFramebuffer> &image, const CBox &box, float alpha,
                         bool cover = false) {
    if (!image || !image->getTexture() || alpha <= 0)
        return;
    const auto local = physical(box.copy().translate(-output->m_position));
    auto &data = g_pHyprRenderer->m_renderData;
    const auto damage = data.damage.copy().intersect(local);
    const auto topLeft = data.primarySurfaceUVTopLeft;
    const auto bottomRight = data.primarySurfaceUVBottomRight;
    const auto restoreUV = makeUnique<Render::CScopeGuard>([topLeft, bottomRight] {
        g_pHyprRenderer->m_renderData.primarySurfaceUVTopLeft = topLeft;
        g_pHyprRenderer->m_renderData.primarySurfaceUVBottomRight = bottomRight;
    });
    if (cover) {
        const auto crop = Enzui::coverCrop(image->m_size.x, image->m_size.y, box.w, box.h);
        if (!crop)
            return;
        data.primarySurfaceUVTopLeft = {0, 0};
        data.primarySurfaceUVBottomRight = {crop->right, crop->bottom};
    }
    g_pHyprRenderer->draw(CTexPassElement::SRenderData{
        .tex = image->getTexture(), .box = local, .a = alpha, .damage = damage,
        .useProvidedDamage = true, .allowCustomUV = cover}, damage);
}
// Draws the app through a picture of it, kept for when it is detached. Glass
// is filtered from the live backdrop, which a picture of the app alone does
// not hold, so an app with glass is drawn live within `damage` and the
// picture is only kept.
static bool drawSmall(Render::CRenderPass &pass, const CRegion &damage) {
    if (!owned() || restoring || additionalWindow() || app->popupMappedTreeSize() != 0 ||
        g_pHyprRenderer->m_bRenderingSnapshot || g_pHyprRenderer->m_bBlockSurfaceFeedback ||
        g_pHyprRenderer->m_renderData.renderingTransformedSource)
        return false;
    const auto box = app->geometricBox(Desktop::View::IGeometric::GEOMETRIC_CURRENT);
    if (box.size() != wanted.size() || box.w > 512 || box.h > 1024 ||
        !geometryOf(app.lock()).matches(box.w, box.h) ||
        !snapshotPass(pass, smallScratch, box))
        return false;
    if (EnzMaterial::hasGlass(app.lock()))
        pass.render(damage);
    else
        paintPicture(smallScratch, box, 1);
    smallPendingBox = box;
    smallPending = true;
    return true;
}
static void updateRestoreProgress(const Time::steady_tp &now) {
    if (!restoreFrame || !restoreFrame->started)
        return;
    const auto elapsed = std::chrono::duration<double>(now - restoreFrame->began).count();
    restoreFrame->progress = Enzui::restoreProgress(restoreFrame->from, elapsed, motionRate);
}
// A window's own drawing, unchanged, at the point it is added: used to draw
// it after the top layers.
class LiftedWindow final : public IPassElement {
    UP<Render::CRenderPass> contents;

  public:
    LiftedWindow(UP<Render::CRenderPass> pass) : contents(std::move(pass)) {}
    bool needsLiveBlur() override { return contents->needsLiveBlur(); }
    bool needsPrecomputeBlur() override { return contents->needsPrecomputeBlur(); }
    bool disableSimplification() override { return true; }
    bool requiresFullDamage() override { return false; }
    const char *passName() override { return "EnzLiftedWindow"; }
    ePassElementType type() override { return EK_CUSTOM; }
    std::vector<UP<IPassElement>> draw() override {
        if (g_pHyprRenderer->m_renderData.pMonitor != output)
            return {};
        const auto saved = g_pHyprRenderer->m_renderData;
        const auto restoreData = makeUnique<Render::CScopeGuard>([saved] {
            g_pHyprRenderer->m_renderData = saved;
        });
        contents->render(saved.damage);
        return {};
    }
};
class RetainedWindow final : public IPassElement {
    UP<Render::CRenderPass> contents;
    PHLWINDOWREF window;
    uint64_t epoch;
  public:
    RetainedWindow(UP<Render::CRenderPass> pass)
        : contents(std::move(pass)), window(app), epoch(ticket) {}
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    bool disableSimplification() override { return true; }
    bool requiresFullDamage() override { return true; }
    const char *passName() override { return "EnzSidebarScene"; }
    ePassElementType type() override { return EK_CUSTOM; }
    std::vector<UP<IPassElement>> draw() override {
        if (!owned() || foreign || window != app || epoch != ticket ||
            g_pHyprRenderer->m_renderData.pMonitor != output)
            return {};
        const auto saved = g_pHyprRenderer->m_renderData;
        const auto restoreData = makeUnique<Render::CScopeGuard>([saved] {
            g_pHyprRenderer->m_renderData = saved;
        });
        if (detached) {
            if (!drawSmall(*contents, saved.damage))
                contents->render(saved.damage);
            return {};
        }
        if (!restoring || !restoreFrame)
            return {};
        auto &frame = *restoreFrame;
        if (!restoredContentReady()) {
            frame.visible = frame.source.box;
            if (rasterSupported())
                paintPicture(frame.source.image, frame.visible, 1);
            if (auto surface = app->backend().surface())
                surface->presentFeedback(Time::steadyNow(), output.lock(), true);
            return {};
        }
        if (!windowMotion || !ownerAlive() || !rasterSupported() || additionalWindow() ||
            frame.progress >= 1) {
            contents->render(saved.damage);
            frame.visible = wanted;
            frame.progress = 1;
            openingArmed = true;
            ++rendered;
            return {};
        }
        if (!snapshotPass(*contents, frame.destination, wanted)) {
            contents->render(saved.damage);
            frame.visible = wanted;
            frame.progress = 1;
            openingArmed = true;
            return {};
        }
        if (!frame.started) {
            frame.started = true;
            frame.began = Time::steadyNow();
        }
        const auto &a = frame.source.box;
        const auto p = frame.progress;
        frame.visible = CBox{std::lerp(a.x, wanted.x, p), std::lerp(a.y, wanted.y, p),
                             std::lerp(a.w, wanted.w, p), std::lerp(a.h, wanted.h, p)}.round();
        paintPicture(frame.destination, frame.visible, 1, true);
        paintPicture(frame.source.image, frame.visible, 1 - std::clamp((p - .25) / .75, 0.0, 1.0), true);
        ++rendered;
        return {};
    }
};
class Marker final : public IPassElement {
  public:
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    bool disableSimplification() override { return true; }
    bool requiresFullDamage() override { return true; }
    const char *passName() override { return "EnzSidebarScene"; }
    ePassElementType type() override { return EK_CUSTOM; }
    std::vector<UP<IPassElement>> draw() override {
        if (restoredContentReady()) {
            openingArmed = true;
            ++rendered;
        }
        return {};
    }
};
class RailPass final : public IPassElement {
    UP<Render::CRenderPass> contents;
    PHLWINDOWREF window;
    CBox box;
    Vector2D shift;
    pid_t owner;
  public:
    bool floating;
    RailPass(UP<Render::CRenderPass> pass, PHLWINDOW w)
        : contents(std::move(pass)), window(w), box(rail.box), shift(railShift()), owner(ownerPid), floating(rail.floating) {}
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    bool disableSimplification() override { return true; }
    // It draws only inside the frame's damage, and every move of the rail
    // damages the output, so a frame elsewhere (a keystroke in an app) need
    // not repaint the whole output.
    bool requiresFullDamage() override { return false; }
    const char *passName() override { return "EnzSidebarRail"; }
    ePassElementType type() override { return EK_CUSTOM; }
    std::optional<CBox> boundingBox() override {
        auto visible = box.copy().translate(shift);
        if (floating && output) visible = visible.intersection(CBox{0, 0, output->m_size.x, output->m_size.y});
        return visible;
    }
    std::vector<UP<IPassElement>> draw() override {
        // While the rail changes size its picture is a frame behind its box.
        // Its last picture stays, fitted to the new box at the screen edge,
        // rather than the rail vanishing for those frames (the desktop
        // showed through).
        if (owner == ownerPid && !railSuppressed() && !railReady() && window && window == railSource() &&
            g_pHyprRenderer->m_renderData.pMonitor == output && !rail.floating) {
            const auto w = window.lock();
            const auto surface = w->backend().surface();
            if (surface && surface->m_current.texture && surface->m_current.size.x >= 1) {
                surface->presentFeedback(Time::steadyNow(), w->m_monitor.lock(), true);
                // The picture keeps the surface's own fade (the shell fades
                // the rail out and in when it changes style, which resizes
                // it): drawn opaque it flashed for those frames.
                const auto view = Desktop::View::CWLSurface::fromResource(surface);
                const float alpha = view ? view->m_alphaModifier * view->m_overallOpacity : 1.F;
                if (alpha <= 0.F)
                    return {};
                std::vector<UP<IPassElement>> picture;
                picture.emplace_back(makeUnique<CTexPassElement>(CTexPassElement::SRenderData{
                    .tex = surface->m_current.texture,
                    .box = CBox{rail.box.pos() + shift, Vector2D{rail.box.w, surface->m_current.size.y}}.scale(output->m_scale).round(),
                    .a = alpha}));
                return picture;
            }
        }
        if (owner != ownerPid || railSuppressed() || !railReady() || window != railSource() || box != rail.box || floating != rail.floating ||
            g_pHyprRenderer->m_renderData.pMonitor != output || contents->single()) {
            if (window && window->backend().surface())
                window->backend().surface()->presentFeedback(Time::steadyNow(), window->m_monitor.lock(), true);
            return {};
        }
        const auto saved = g_pHyprRenderer->m_renderData;
        if (contents->needsLiveBlur() || contents->needsPrecomputeBlur() || !saved.renderModif.modifs.empty())
            return {};
        const auto delta = shift * output->m_scale;
        auto clip = saved.damage.copy().intersect(box.copy().translate(shift).scale(output->m_scale).round());
        if (floating)
            clip.intersect(CBox{0, 0, output->m_size.x, output->m_size.y}.scale(output->m_scale).round());
        if (clip.empty())
            return {};
        const bool blur = output->m_blurFBShouldRender;
        auto &data = g_pHyprRenderer->m_renderData;
        data.noSimplify = true;
        data.renderModif = {};
        if (delta.x != 0)
            data.renderModif.modifs.emplace_back(std::make_pair(Render::SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, delta));
        contents->render(clip.copy().translate(-delta));
        data = saved;
        output->m_blurFBShouldRender = blur;
        return {};
    }
};
class PocketPass final : public IPassElement {
    UP<Render::CRenderPass> contents;
    PHLWINDOWREF window;
    CBox box;
    double fraction;
    bool onRight;
    Vector2D shift;
    pid_t owner;
  public:
    PocketPass(UP<Render::CRenderPass> pass, PHLWINDOW w)
        : contents(std::move(pass)), window(w), box(pocket.box), fraction(pocket.progress),
          onRight(pocket.right), shift(railShift()), owner(ownerPid) {}
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    bool disableSimplification() override { return true; }
    bool requiresFullDamage() override { return true; }
    const char *passName() override { return "EnzSidebarPocket"; }
    ePassElementType type() override { return EK_CUSTOM; }
    std::optional<CBox> boundingBox() override { return box.copy().translate(shift); }
    std::vector<UP<IPassElement>> draw() override {
        if (owner != ownerPid || !pocketRenderable() || pocket.box != box ||
            g_pHyprRenderer->m_renderData.pMonitor != output || !backingReadyFor(box) ||
            backing != window || contents->single())
            return {};
        auto saved = g_pHyprRenderer->m_renderData;
        if (contents->needsLiveBlur() || contents->needsPrecomputeBlur() ||
            !saved.renderModif.modifs.empty())
            return {};
        const auto clipped = saved.damage.copy().intersect(box.copy().translate(shift).scale(output->m_scale).round());
        if (clipped.empty())
            return {};
        const Vector2D delta = Vector2D{(onRight ? 1 : -1) * std::round(box.w * (1 - fraction) * output->m_scale), 0.0} + shift * output->m_scale;
        const bool blur = output->m_blurFBShouldRender;
        auto &data = g_pHyprRenderer->m_renderData;
        data.noSimplify = true;
        data.renderModif = {};
        if (delta.x != 0)
            data.renderModif.modifs.emplace_back(std::make_pair(Render::SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, delta));
        contents->render(clipped.copy().translate(-delta));
        data = saved;
        output->m_blurFBShouldRender = blur;
        return {};
    }
};
class Group final : public IPassElement {
    UP<Render::CRenderPass> contents, background;
    PHLWINDOWREF window;
    double value;
    Vector2D shift;
    CBox visible;
    float magnification = 1;
    bool toIcon;

  public:
    Group(UP<Render::CRenderPass> pass)
        : contents(std::move(pass)), background(std::move(backingPass)), window(app),
          value(progress), shift(railShift()), visible(pane.copy().translate(shift)),
          toIcon(rail.floating && iconAnchor.has_value()) {
        if (toIcon) {
            magnification = static_cast<float>(std::lerp(4.0 / std::max(pane.w, pane.h), 1.0, value));
            const auto center = *iconAnchor * (1 - value) + (pane.pos() + pane.size() / 2) * value + shift;
            visible = {center - pane.size() * magnification / 2, pane.size() * magnification};
        }
    }
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    bool disableSimplification() override { return true; }
    // It draws only inside the frame's damage; its moves damage the output
    // and the app's commits its pane, so an open app need not repaint the
    // whole output on each of them.
    bool requiresFullDamage() override { return false; }
    const char *passName() override { return "EnzSidebarScene"; }
    ePassElementType type() override { return EK_CUSTOM; }
    // The open miniapp casts the same soft shadow as a window, around its pane.
    std::optional<CBox> boundingBox() override { return visible.copy().expand(shadowRange()); }
    static double shadowRange() {
        static auto range = CConfigValue<Config::INTEGER>("decoration:shadow:range");
        return std::clamp<double>(*range, 0.0, 64.0);
    }
    std::vector<UP<IPassElement>> draw() override {
        if (!owned() || window != app || foreign || detached ||
            g_pHyprRenderer->m_renderData.pMonitor != output)
            return {};
        if (contents->single()) {
            fail("empty-pass");
            return {};
        }
        if (!gate.ready() || failed || value == 0 || !background || !backingReady() || !railReady()) {
            if (resizeBackdropActive() && value == 1) {
                contents->render(g_pHyprRenderer->m_renderData.damage);
                ++rendered;
                return {};
            }
            if (auto s = app->backend().surface())
                s->presentFeedback(Time::steadyNow(), output.lock(), true);
            if (backing && backing->backend().surface())
                backing->backend().surface()->presentFeedback(Time::steadyNow(), output.lock(),
                                                              true);
            if (goal == 0 && value == 0)
                closingArmed = true;
            return {};
        }
        auto saved = g_pHyprRenderer->m_renderData;
        if (contents->needsLiveBlur() || contents->needsPrecomputeBlur() ||
            background->needsLiveBlur() || background->needsPrecomputeBlur() ||
            !saved.renderModif.modifs.empty()) {
            fail("unsupported-render");
            return {};
        }
        const auto clipped =
            saved.damage.copy().intersect(visible.copy().scale(output->m_scale).round());
        if (clipped.empty())
            return {};
        const Vector2D delta = toIcon
            ? (visible.pos() - pane.pos() * magnification) * output->m_scale
            : Vector2D{(right ? 1 : -1) * std::round(pane.w * (1 - value) * output->m_scale), 0.0} + shift * output->m_scale;
        const bool blur = output->m_blurFBShouldRender;
        if (const double range = shadowRange(); range * magnification * output->m_scale >= 1) {
            auto &data = g_pHyprRenderer->m_renderData;
            data = saved;
            const double scale = output->m_scale;
            CBox box;
            if (toIcon) {
                box = visible.copy().expand(range * magnification).scale(scale).round();
                data.damage.intersect(box);
            } else {
                // No shadow over the rail the pane grows out of.
                box = pane.copy().translate(shift).expand(range);
                box.w -= range;
                if (!right)
                    box.x += range;
                data.damage.intersect(box.copy().scale(scale).round());
                box = box.scale(scale).translate(delta).round();
            }
            static auto rounding = CConfigValue<Config::INTEGER>("decoration:rounding");
            const auto radius = static_cast<int>(std::round(std::clamp<double>(*rounding, 0.0, 48.0) * scale * magnification));
            g_pHyprRenderer->disableScissor();
            g_pHyprRenderer->blend(true);
            g_pHyprRenderer->drawShadow(box, radius, 2.0F, static_cast<int>(std::round(range * scale * magnification)),
                                        Config::CGradientValueData{CHyprColor(0.0, 0.0, 0.0, 0.28)},
                                        static_cast<float>(value));
        }
        for (auto *pass : {background.get(), contents.get()}) {
            auto &data = g_pHyprRenderer->m_renderData;
            data = saved;
            data.noSimplify = true;
            data.renderModif = {};
            if (toIcon && magnification != 1)
                data.renderModif.modifs.emplace_back(std::make_pair(
                    Render::SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE, magnification));
            if (delta.x != 0 || delta.y != 0)
                data.renderModif.modifs.emplace_back(std::make_pair(
                    Render::SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, delta));
            if (pass != contents.get() || value != 1 || goal != 1 || animating || shift.x != 0 ||
                !drawSmall(*pass, clipped))
                pass->render(clipped.copy().translate(-delta).scale(1.0 / magnification));
        }
        g_pHyprRenderer->m_renderData = saved;
        output->m_blurFBShouldRender = blur;
        if (value == 1 && goal == 1 && !animating && railInputReady())
            openingArmed = true;
        ++rendered;
        return {};
    }
};
static int result(lua_State *L, bool value) {
    lua_pushboolean(L, value);
    return 1;
}
static bool motionOwner(lua_State *L) {
    return lua_isinteger(L, 1) && lua_tointeger(L, 1) == ownerPid && ownerAlive() &&
           output && output->enabled() && !remote(bar.lock());
}
#include "ShellRegions.inc"
#include "SidebarViewport.inc"
#include "SidebarDrag.inc"
#include "TileDrop.inc"
#include "IconFeedback.inc"
#include "SidebarDetachTab.inc"
#include "WorkspaceTransition.inc"
#include "OverviewBackdrop.inc"
#include "PanelShape.inc"
#include "SearchDim.inc"
#include "SidebarLuaApi.inc"
#include "SidebarLifecycle.inc"
} // namespace SidebarScene
#undef g_pHyprRenderer
#undef g_pSeatManager
#undef g_pInputManager
#undef g_layoutManager
