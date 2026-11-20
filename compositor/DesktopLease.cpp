#include "DesktopLease.hpp"
#include "../vendor/json/single_include/nlohmann/json.hpp"
#include "src/Compositor.hpp"
#include "src/config/ConfigManager.hpp"
#include "src/config/lua/ConfigManager.hpp"
#include "src/desktop/state/WindowState.hpp"
#include "src/desktop/state/FocusState.hpp"
#include "src/desktop/view/window/WindowPresentation.hpp"
#include "src/desktop/view/window/WindowFullscreenPolicy.hpp"
#include "src/state/MonitorState.hpp"
#include "src/managers/fullscreen/FullscreenController.hpp"
#include "src/managers/input/InputManager.hpp"
#include "src/layout/algorithm/Algorithm.hpp"
#include "src/layout/target/WindowTarget.hpp"
#include "src/output/Monitor.hpp"
#include "src/render/Renderer.hpp"
#include <aquamarine/backend/Backend.hpp>
#include <aquamarine/output/Output.hpp>
#include <sys/random.h>
#include <sys/stat.h>
#include <unistd.h>
#include <algorithm>
#include <cmath>
#include <format>

namespace Enzremote {
using Json = nlohmann::json;
using Geometry = Desktop::View::IGeometric;

static std::string token() {
    unsigned char bytes[16]; size_t used = 0;
    while (used < sizeof(bytes)) {
        auto n = getrandom(bytes + used, sizeof(bytes) - used, GRND_NONBLOCK);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) throw std::runtime_error("Cannot generate lease identity");
        used += n;
    }
    std::string result;
    for (auto byte : bytes) result += std::format("{:02x}", byte);
    return result;
}

static bool equalToken(const std::string& a, const std::string& b) {
    if (a.size() != b.size()) return false;
    unsigned mismatch = 0; for (size_t i = 0; i < a.size(); ++i) mismatch |= a[i] ^ b[i];
    return !mismatch;
}

static void lua(const std::string& code) {
    auto manager = dynamic_cast<Config::Lua::CConfigManager*>(Config::mgr().get());
    if (!manager) throw std::runtime_error("Shared desktop requires the Lua desktop configuration");
    if (auto error = manager->eval(code)) throw std::runtime_error(*error);
}

static std::string selector(PHLWINDOW window) { return Json(std::format("address:0x{:x}", reinterpret_cast<uintptr_t>(window.get()))).dump(); }
static void dispatch(const std::string& expression) {
    lua("local r=hl.dispatch(" + expression + "); if r and r.ok==false then error(tostring(r.error)) end");
}
static CBox box(PHLWINDOW window) { return {window->position(Geometry::GEOMETRIC_GOAL), window->size(Geometry::GEOMETRIC_GOAL)}; }
static bool closeSize(Vector2D a, Vector2D b) { return std::abs(a.x-b.x) < 1.1 && std::abs(a.y-b.y) < 1.1; }

CDesktopLease::CDesktopLease(PHLWINDOW window, SViewport viewport, CRemoteSeat& seat) : m_window(window), m_seat(seat), m_viewport(viewport) {
    const auto surface = window ? window->backend().surface() : nullptr;
    if (!window || !surface || !window->mapped() || !window->m_workspace || !window->m_monitor || window->backend().isX11())
        throw std::runtime_error("Window is not a mapped native desktop application");
    if ((window->m_state & Desktop::View::WINDOW_STATE_PINNED) || window->isHidden()) throw std::runtime_error("Pinned or hidden windows cannot be handed off");
    m_originalBox = m_restoreBox = box(window);
    m_contentHeight = viewport.height;
    m_originalWorkspace = window->m_workspace;
    m_originalMonitor = window->m_monitor;
    m_originalFloating = m_restoreFloating = window->isFloating();
    auto modes = Fullscreen::controller()->getFullscreenModes(window);
    m_originalInternalFS = m_restoreInternalFS = modes.internal;
    m_originalClientFS = m_restoreClientFS = modes.client;
    m_id = token(); m_uiToken = token(); m_inputToken = token();
    m_mobileOutput = "ENZREMOTE-APP"; m_mobileWorkspace = "enzremote-mobile"; m_stagingWorkspace = "enzremote-stage";
    m_directory = g_pCompositor->m_instancePath + "/remote-lease-" + m_id;
    m_snapshotPath = m_directory + "/snapshot.png";
    if (mkdir(m_directory.c_str(), 0700) < 0) throw std::runtime_error("Cannot create private lease directory");
    m_deadline = std::chrono::steady_clock::now() + std::chrono::seconds(8);
    m_committed = surface->m_events.commit.listen([this, weak=WP<CWLSurfaceResource>(surface)]() {
        if (weak && weak->m_current.updated.bits.buffer) ++m_bufferCommits;
    });
}

CDesktopLease::~CDesktopLease() {
    try { if (m_moved && m_window) finishRestore(false); cleanupOutput(); } catch (...) { LOG(Log::ERR, "[enzremote] Lease cleanup failed"); }
    if (m_placeholder && m_placeholder->mapped()) m_placeholder->backend().close();
    unlink(m_snapshotPath.c_str()); rmdir(m_directory.c_str());
}

void CDesktopLease::prepare() {
    for (const auto& candidate : Desktop::windowState()->windows()) {
        if (!candidate || candidate==m_window.lock() || !candidate->mapped() || !ownsWindow(candidate)) continue;
        const auto modes=Fullscreen::controller()->getFullscreenModes(candidate);
        if (!candidate->isFloating() || candidate->isHidden() || modes.internal!=0 || modes.client!=0 || candidate->m_workspace!=m_originalWorkspace)
            throw std::runtime_error("Transient window state requires desktop view");
        if (m_dialogs.size()>=16) throw std::runtime_error("Too many transient windows");
        SDialog dialog; dialog.window=candidate; dialog.desktopSize=box(candidate).size();
        dialog.desktopOffset=box(candidate).pos()-m_originalBox.pos(); dialog.existing=true;
        dialog.allowedOverFullscreen=candidate->fullscreenPolicy().allowedOverFullscreen();
        m_dialogs.push_back(dialog);
    }
    if (State::monitorState()->query().name(m_mobileOutput).run()) throw std::runtime_error("Remote output name is already in use");
    auto monitor = m_originalMonitor.lock();
    if (!monitor) throw std::runtime_error("Desktop output disappeared");
    lua("hl.workspace_rule({workspace=" + Json("name:"+m_stagingWorkspace).dump()+",monitor="+Json(monitor->m_name).dump()+"})");
    lua("hl.window_rule({name=\"enzremote-placeholder\",match={class=\"^dev[.]enzui[.]RemotePlaceholder$\"},float=true,workspace="+
        Json("name:"+m_stagingWorkspace+" silent").dump()+",no_initial_focus=true,no_anim=true})");
    lua("hl.window_rule({name=\"enzremote-transfer-guard\",match={tag="+Json("enzremote-guard-"+m_id).dump()+"},no_anim=true,no_focus=true})");
    lua("hl.window_rule({name=\"enzremote-mobile-style\",match={tag="+Json("enzremote-mobile-"+m_id).dump()+"},border_size=0,rounding=0,decorate=false,no_shadow=true})");
    configureOutput();
    for (const auto& implementation : g_pCompositor->m_aqBackend->getImplementations()) {
        if (implementation->type() != Aquamarine::AQ_BACKEND_HEADLESS) continue;
        implementation->createOutput(m_mobileOutput); m_createdOutput = true; break;
    }
    if (!m_createdOutput) throw std::runtime_error("No headless backend is available in this desktop");
}

void CDesktopLease::resetFrameBarrier() {
    m_stableFrames=0; m_matchingCommits=0; m_observedCommit=m_bufferCommits;
}

bool CDesktopLease::frameReady(bool matches) {
    if (!matches) { resetFrameBarrier(); return false; }
    ++m_stableFrames;
    if (m_observedCommit!=m_bufferCommits) { ++m_matchingCommits; m_observedCommit=m_bufferCommits; }
    // A correctly sized first buffer can still contain the previous responsive layout.
    // Static clients may submit only once, so keep pumping callbacks through a short settle window.
    return m_stableFrames>=2 && (m_matchingCommits>=2 || m_stableFrames>=6);
}

void CDesktopLease::configureOutput() {
    double right = 0;
    for (const auto& monitor : State::monitorState()->allMonitors()) if (monitor->m_name != m_mobileOutput)
        right = std::max(right, monitor->m_position.x + monitor->m_size.x);
    lua(std::format("hl.monitor({{output={},mode=\"{}x{}@{}\",position=\"{}x0\",scale={},reserved={{top=0,bottom=0,left=0,right=0}}}})",
        Json(m_mobileOutput).dump(), m_viewport.width, m_viewport.height, m_viewport.refresh, static_cast<int>(right+8192), m_viewport.scale));
    lua("hl.workspace_rule({workspace="+Json("name:"+m_mobileWorkspace).dump()+",monitor="+Json(m_mobileOutput).dump()+",default=true})");
}

void CDesktopLease::tag(PHLWINDOW w, const std::string& tagName) { dispatch("hl.dsp.window.tag({window="+selector(w)+",tag="+Json(tagName).dump()+"})"); }
void CDesktopLease::move(PHLWINDOW w, const std::string& workspace) {
    dispatch("hl.dsp.window.move({window="+selector(w)+",workspace="+Json(workspace).dump()+",follow=false})");
    auto& presentation=w->presentation();
    presentation.alpha(Desktop::View::WINDOW_ALPHA_MOVE_TO_WORKSPACE)->resetAllCallbacks();
    presentation.alpha(Desktop::View::WINDOW_ALPHA_MOVE_TO_WORKSPACE)->setValueAndWarp(1.F);
    presentation.alpha(Desktop::View::WINDOW_ALPHA_MOVE_FROM_WORKSPACE)->setValueAndWarp(1.F);
    presentation.resetMonitorMovedFrom();
}
void CDesktopLease::floating(PHLWINDOW w, bool enabled) { dispatch("hl.dsp.window.float({window="+selector(w)+",action="+Json(enabled?"on":"off").dump()+"})"); }
void CDesktopLease::fullscreen(PHLWINDOW w, int internal, int client) {
    auto current=Fullscreen::controller()->getFullscreenModes(w);
    if (current.internal==internal && current.client==client) return;
    dispatch(std::format("hl.dsp.window.fullscreen_state({{window={},internal={},client={},action=\"set\"}})",selector(w),internal,client));
}
void CDesktopLease::geometry(PHLWINDOW w, const CBox& b) {
    dispatch(std::format("hl.dsp.window.resize({{window={},x={},y={}}})",selector(w),b.w,b.h));
    dispatch(std::format("hl.dsp.window.move({{window={},x={},y={}}})",selector(w),b.x,b.y));
    w->layoutTarget()->warpPositionSize();
    if (auto target=dynamic_cast<Layout::CWindowTarget*>(w->layoutTarget().get())) target->sendWindowSize(true);
}

bool CDesktopLease::matchesFrame(PHLWINDOW w, Vector2D logical, double scale) const {
    if (!w || !w->mapped()) return false;
    auto surface = w->backend().surface();
    return surface && closeSize(surface->m_current.size, logical) && closeSize(surface->m_current.bufferSize, logical*scale);
}

void CDesktopLease::bindUI(int pid) {
    if (m_uiPid != -1 || m_phase != Phase::PREPARING) throw std::runtime_error("Placeholder has already been registered");
    m_uiPid = pid;
}
bool CDesktopLease::acceptsUI(const std::string& value) const { return m_phase == Phase::PREPARING && m_uiPid == -1 && equalToken(value,m_uiToken); }
bool CDesktopLease::acceptsInput(const std::string& value) const { return m_phase == Phase::ACTIVE && equalToken(value,m_inputToken); }
bool CDesktopLease::inputAllowed() const {
    return m_phase == Phase::ACTIVE && std::ranges::all_of(m_dialogs, [](const auto& dialog) {
        return !dialog.window || !dialog.window->mapped() || dialog.ready;
    });
}
bool CDesktopLease::finished() const { return m_phase == Phase::CLOSED || m_phase == Phase::FAILED || m_phase == Phase::RESTORED; }
bool CDesktopLease::ownsWindow(PHLWINDOW candidate) const {
    auto root=m_window.lock();
    if (!root || !candidate || candidate->backend().isX11()) return false;
    auto source=root->backend().surface(); auto surface=candidate->backend().surface();
    if (!source || !surface || source->client()!=surface->client()) return false;
    for (unsigned depth=0; candidate && depth<32; ++depth) {
        if (candidate==root || (m_phase!=Phase::PREPARING && std::ranges::any_of(m_dialogs,[&](const auto& dialog) { return dialog.window.lock()==candidate; }))) return true;
        candidate=candidate->backend().parent();
    }
    return false;
}

std::vector<PHLWINDOW> CDesktopLease::inputWindows() const {
    std::vector<PHLWINDOW> result;
    if (m_window && m_window->mapped()) result.push_back(m_window.lock());
    for (const auto& dialog : m_dialogs)
        if (dialog.ready && dialog.window && dialog.window->mapped()) result.push_back(dialog.window.lock());
    return result;
}

PHLWINDOW CDesktopLease::inputWindow() const {
    if (m_inputWindow && m_inputWindow->mapped()) return m_inputWindow.lock();
    for (auto it=m_dialogs.rbegin(); it!=m_dialogs.rend(); ++it)
        if (it->ready && it->window && it->window->mapped()) return it->window.lock();
    return m_window.lock();
}

void CDesktopLease::focusInput(PHLWINDOW window) {
    if (!window || !window->mapped() || !ownsWindow(window)) return;
    if (m_inputWindow.lock()==window && m_seat.ready()) return;
    if (m_inputWindow && m_inputWindow->mapped() && Desktop::focusState()->window()!=m_inputWindow.lock()) m_inputWindow->backend().setActive(false);
    m_inputWindow=window; window->backend().setActive(true); m_seat.focus(window->backend().surface());
}

void CDesktopLease::windowMapped(PHLWINDOW window) {
    if (finished() || m_phase==Phase::PREPARING || window==m_window.lock() || !window || !window->mapped() || !ownsWindow(window)) return;
    if (std::ranges::any_of(m_dialogs,[&](const auto& dialog) { return dialog.window.lock()==window; })) return;
    std::erase_if(m_dialogs,[](const auto& dialog) { return !dialog.window || !dialog.window->mapped(); });
    if (m_dialogs.size()>=16) { restore("Too many transient windows"); return; }
    SDialog dialog; dialog.window=window; dialog.desktopSize=box(window).size();
    m_dialogs.push_back(dialog);
    tag(window,"+enzremote-guard-"+m_id);
    fullscreen(window,0,0); floating(window,true);
    window->fullscreenPolicy().setAllowedOverFullscreen(true);
    placeDialogs(m_phase==Phase::RESTORING);
}

void CDesktopLease::placeDialogs(bool restoring) {
    const auto area=restoring ? m_restoreBox.size() : Vector2D{static_cast<double>(m_viewport.width),static_cast<double>(m_contentHeight)}/m_viewport.scale;
    const auto origin=restoring ? m_restoreBox.pos() : (m_remoteMonitor ? m_remoteMonitor->m_position : Vector2D{});
    for (auto& dialog : m_dialogs) {
        if (!dialog.window || !dialog.window->mapped()) continue;
        auto w=dialog.window.lock();
        const auto size=restoring ? dialog.desktopSize : Vector2D{std::min(dialog.desktopSize.x,area.x),std::min(dialog.desktopSize.y,area.y)};
        if (restoring || !dialog.ready || !closeSize(size,dialog.target.size())) {
            dialog.ready=false; dialog.stable=0;
            dialog.deadline=std::chrono::steady_clock::now()+std::chrono::seconds(4);
        }
        const auto position=restoring && dialog.existing ? origin+dialog.desktopOffset : origin+(area-size)/2.0;
        dialog.target=CBox{position,size};
        move(w,"name:"+(restoring ? m_stagingWorkspace : m_mobileWorkspace));
        geometry(w,dialog.target);
        // setHidden is overridden by layout and suspends client redraws; alpha stages the buffer instead.
        if (!dialog.ready) {
            w->presentation().alpha(Desktop::View::WINDOW_ALPHA_MOVE_FROM_WORKSPACE)->setValueAndWarp(0.F);
            w->setSuspended(false);
        }
    }
}

bool CDesktopLease::settleDialogs(bool restoring) {
    bool ready=true;
    for (auto& dialog : m_dialogs) {
        if (!dialog.window || !dialog.window->mapped() || dialog.ready) continue;
        auto w=dialog.window.lock();
        w->setSuspended(false);
        if (auto surface=w->backend().surface()) surface->frame(Time::steadyNow());
        const auto scale=restoring && m_originalMonitor ? m_originalMonitor->m_scale : m_viewport.scale;
        if (matchesFrame(w,dialog.target.size(),scale)) ++dialog.stable; else dialog.stable=0;
        if (dialog.stable>=6) {
            dialog.ready=true;
            if (!restoring) {
                w->presentation().alpha(Desktop::View::WINDOW_ALPHA_MOVE_FROM_WORKSPACE)->setValueAndWarp(1.F);
                focusInput(w);
            }
        } else {
            ready=false;
            if (!restoring && std::chrono::steady_clock::now()>dialog.deadline) {
                if (auto surface=w->backend().surface()) LOG(Log::ERR,"[enzremote] Transient frame target {} surface {} buffer {} scale {}",dialog.target.size(),surface->m_current.size,surface->m_current.bufferSize,scale);
                restore("Transient window did not fit the mobile viewport"); return false;
            }
        }
    }
    return ready;
}

void CDesktopLease::returnDialogs() {
    for (auto& dialog : m_dialogs) {
        if (!dialog.window || !dialog.window->mapped()) continue;
        auto w=dialog.window.lock();
        if (m_originalWorkspace) {
            move(w,m_originalWorkspace->getConfigName());
            const auto offset=dialog.existing ? dialog.desktopOffset : (m_restoreBox.size()-dialog.desktopSize)/2.0;
            geometry(w,{m_restoreBox.pos()+offset,dialog.desktopSize});
        }
        if (dialog.existing) w->fullscreenPolicy().setAllowedOverFullscreen(dialog.allowedOverFullscreen);
        tag(w,"-enzremote-guard-"+m_id); w->backend().setActive(false);
        Desktop::windowState()->raise(w);
    }
}

bool CDesktopLease::uiReady() const { return m_placeholder && m_originalMonitor && matchesFrame(m_placeholder.lock(), m_originalBox.size(), m_originalMonitor->m_scale); }
PHLWINDOW CDesktopLease::window() const { return m_window.lock(); }
PHLMONITOR CDesktopLease::remoteMonitor() const { return m_remoteMonitor.lock(); }
CDesktopLease::Phase CDesktopLease::phase() const { return m_phase; }

void CDesktopLease::activate() {
    if (m_phase != Phase::PREPARING || !uiReady() || !m_remoteMonitor) throw std::runtime_error("Placeholder or output is not ready");
    auto w = m_window.lock(); auto p = m_placeholder.lock();
    for (const auto& dialog : m_dialogs)
        if (dialog.window && dialog.window->mapped() && !ownsWindow(dialog.window.lock()))
            throw std::runtime_error("Transient window parent changed during handoff preparation");
    for (const auto& candidate : Desktop::windowState()->windows()) {
        if (!candidate || candidate==w || !candidate->mapped() || !ownsWindow(candidate)) continue;
        const auto found=std::ranges::find_if(m_dialogs,[&](const auto& dialog) { return dialog.window.lock()==candidate; });
        const auto modes=Fullscreen::controller()->getFullscreenModes(candidate);
        if (found==m_dialogs.end() || modes.internal!=0 || modes.client!=0 || !candidate->isFloating() || candidate->isHidden() || candidate->m_workspace!=m_originalWorkspace ||
            !closeSize(box(candidate).size(),found->desktopSize) || !closeSize(box(candidate).pos(),m_originalBox.pos()+found->desktopOffset))
            throw std::runtime_error("Transient window changed during handoff preparation");
    }
    auto localFocus=Desktop::focusState()->window();
    if (ownsWindow(localFocus)) localFocus=p;
    if (!w || !w->mapped() || !closeSize(box(w).pos(),m_originalBox.pos()) || !closeSize(box(w).size(),m_originalBox.size()) || w->m_workspace != m_originalWorkspace)
        throw std::runtime_error("Desktop window changed during handoff preparation");
    move(p,m_originalWorkspace->getConfigName());
    floating(p,m_originalFloating);
    if (m_originalFloating) geometry(p,m_originalBox);
    else m_originalWorkspace->m_space->algorithm()->swapTargets(w->layoutTarget(),p->layoutTarget());
    if (ownsWindow(Desktop::focusState()->window())) Desktop::focusState()->fullWindowFocus(p,Desktop::FOCUS_REASON_DESKTOP_STATE_CHANGE);
    Desktop::windowState()->swapOrder(w,p);
    tag(w,"+enzremote-guard-"+m_id);
    m_moved = true;
    fullscreen(w,0,0);
    floating(w,true);
    fullscreen(p,m_originalInternalFS,m_originalClientFS);
    tag(w,"+enzremote-mobile-"+m_id);
    move(w,"name:"+m_mobileWorkspace);
    m_remoteMonitor->changeWorkspace(w->m_workspace,true,true,true);
    fullscreen(w,1,2);
    w->layoutTarget()->warpPositionSize();
    m_phase = Phase::CONFIGURING; resetFrameBarrier();
    for (auto& dialog : m_dialogs) if (dialog.window && dialog.window->mapped()) {
        tag(dialog.window.lock(),"+enzremote-guard-"+m_id);
        dialog.window->fullscreenPolicy().setAllowedOverFullscreen(true);
    }
    placeDialogs(false);
    m_deadline = std::chrono::steady_clock::now()+std::chrono::seconds(4);
    if (localFocus && localFocus->mapped()) {
        Desktop::windowState()->raise(localFocus);
        Desktop::focusState()->fullWindowFocus(localFocus,Desktop::FOCUS_REASON_DESKTOP_STATE_CHANGE);
    }
    g_pInputManager->simulateMouseMovement();
}

void CDesktopLease::resize(SViewport viewport) {
    if (m_phase != Phase::ACTIVE) throw std::runtime_error("App is not ready to resize");
    m_seat.releaseInput(); m_viewport=viewport; m_contentHeight=viewport.height; configureOutput();
    for (auto& dialog : m_dialogs) dialog.ready=false;
    placeDialogs(false);
    m_phase=Phase::CONFIGURING; resetFrameBarrier();
    m_deadline=std::chrono::steady_clock::now()+std::chrono::seconds(4);
}

void CDesktopLease::contentSize(uint32_t width, uint32_t height, double scale) {
    if (m_phase!=Phase::ACTIVE || !m_remoteMonitor || width!=m_viewport.width || height<240 || height>m_viewport.height ||
        !std::isfinite(scale) || std::abs(scale-m_viewport.scale)>.001)
        throw std::runtime_error("Content size does not match the leased canvas");
    const double bottom=(m_viewport.height-height)/scale;
    if (std::abs(bottom-std::round(bottom))>.001) throw std::runtime_error("Content size must align to logical pixels");
    if (!closeSize(m_remoteMonitor->m_pixelSize,{static_cast<double>(m_viewport.width),static_cast<double>(m_viewport.height)}))
        throw std::runtime_error("Leased output geometry changed");
    lua(std::format("hl.monitor({{output={},mode=\"{}x{}@{}\",position=\"{}x{}\",scale={},reserved={{top=0,bottom={},left=0,right=0}}}})",
        Json(m_mobileOutput).dump(),m_viewport.width,m_viewport.height,m_viewport.refresh,
        static_cast<int>(m_remoteMonitor->m_position.x),static_cast<int>(m_remoteMonitor->m_position.y),scale,static_cast<int>(std::round(bottom))));
    m_contentHeight=height; placeDialogs(false);
}

void CDesktopLease::restore(const std::string& reason) {
    if (finished() || m_phase == Phase::RESTORING) return;
    m_reason=reason; m_seat.clear(); m_inputWindow.reset();
    m_restoreFocus=Desktop::focusState()->window();
    m_phase=Phase::RESTORING; resetFrameBarrier();
    m_deadline=std::chrono::steady_clock::now()+std::chrono::seconds(4);
    if (!m_moved || !m_window || !m_window->mapped()) { m_phase=Phase::CLOSED; return; }
    auto w=m_window.lock(); auto p=m_placeholder.lock();
    if (p && p->mapped()) {
        m_restoreBox=box(p); m_restoreFloating=p->isFloating();
        auto modes=Fullscreen::controller()->getFullscreenModes(p);
        m_restoreInternalFS=modes.internal; m_restoreClientFS=modes.client;
        m_originalWorkspace=p->m_workspace; m_originalMonitor=p->m_monitor;
    }
    fullscreen(w,0,0); tag(w,"-enzremote-mobile-"+m_id);
    move(w,"name:"+m_stagingWorkspace); floating(w,true); geometry(w,m_restoreBox);
    fullscreen(w,m_restoreInternalFS,m_restoreClientFS);
    placeDialogs(true);
    m_phase=Phase::RESTORING; resetFrameBarrier();
    m_deadline=std::chrono::steady_clock::now()+std::chrono::seconds(4);
}

void CDesktopLease::finishRestore(bool settled) {
    auto w=m_window.lock(); auto p=m_placeholder.lock();
    if (!w || !w->mapped()) { m_moved=false; m_phase=Phase::CLOSED; return; }
    auto localFocus=Desktop::focusState()->window();
    if (!localFocus || localFocus==p) localFocus=m_restoreFocus.lock();
    if (localFocus==p) localFocus=w;
    if (p && p->mapped()) fullscreen(p,0,0);
    fullscreen(w,0,0);
    tag(w,"-enzremote-mobile-"+m_id);
    move(w,m_originalWorkspace->getConfigName()); floating(w,m_restoreFloating);
    if (m_restoreFloating) geometry(w,m_restoreBox);
    else if (p && p->mapped() && !p->isFloating()) m_originalWorkspace->m_space->algorithm()->swapTargets(w->layoutTarget(),p->layoutTarget());
    fullscreen(w,m_restoreInternalFS,m_restoreClientFS);
    if (p && p->mapped()) Desktop::windowState()->swapOrder(w,p);
    tag(w,"-enzremote-guard-"+m_id);
    returnDialogs();
    if (localFocus==w) {
        for (auto it=m_dialogs.rbegin(); it!=m_dialogs.rend(); ++it)
            if (it->window && it->window->mapped()) { localFocus=it->window.lock(); break; }
    }
    m_moved=false; m_phase=Phase::RESTORED;
    if (localFocus && localFocus->mapped()) {
        Desktop::windowState()->raise(localFocus);
        Desktop::focusState()->fullWindowFocus(localFocus,Desktop::FOCUS_REASON_DESKTOP_STATE_CHANGE);
    }
    if (localFocus!=w) w->backend().setActive(false);
    if (p && p->mapped()) { move(p,"name:"+m_stagingWorkspace); p->backend().close(); }
    m_moved=false; m_phase=Phase::RESTORED;
    if (!settled) m_reason="Restored geometry after app frame timeout";
    cleanupOutput();
    g_pInputManager->simulateMouseMovement();
}

void CDesktopLease::cleanupOutput() {
    if (!m_createdOutput) return;
    auto monitor=m_remoteMonitor.lock();
    if (!monitor) monitor=State::monitorState()->query().name(m_mobileOutput).run();
    if (monitor && monitor->m_createdByUser) monitor->m_output->destroy();
    m_remoteMonitor.reset(); m_createdOutput=false;
}

void CDesktopLease::windowClosed() { m_seat.clear(); returnDialogs(); m_moved=false; m_phase=Phase::CLOSED; m_reason="Application closed"; }

void CDesktopLease::tick() {
    if (finished()) {
        cleanupOutput();
        if (m_placeholder && m_placeholder->mapped()) m_placeholder->backend().close();
        m_placeholder.reset();
        unlink(m_snapshotPath.c_str()); rmdir(m_directory.c_str());
        return;
    }
    if (!m_window || !m_window->mapped()) { windowClosed(); return; }
    if (!m_remoteMonitor) m_remoteMonitor=State::monitorState()->query().name(m_mobileOutput).run();
    if (m_phase==Phase::ACTIVE && (!m_remoteMonitor || !m_remoteMonitor->enabled() || !m_remoteMonitor->m_output)) {
        restore("Remote output disappeared"); return;
    }
    if (m_phase==Phase::PREPARING && m_uiPid>0 && !m_placeholder) {
        for (auto& w : Desktop::windowState()->windows())
            if (w && w->mapped() && w->backend().pid()==m_uiPid && w->metadata().appID()=="dev.enzui.RemotePlaceholder") { m_placeholder=w; geometry(w,m_originalBox); break; }
    }
    const bool dialogsReady=m_phase==Phase::PREPARING || settleDialogs(m_phase==Phase::RESTORING);
    if (m_phase==Phase::ACTIVE && (!m_inputWindow || !m_inputWindow->mapped())) focusInput(inputWindow());
    // Hidden staging must complete frame callbacks even if the previous output disappeared.
    if (m_phase==Phase::CONFIGURING || m_phase==Phase::RESTORING) {
        m_window->setSuspended(false);
        if (auto surface=m_window->backend().surface()) surface->frame(Time::steadyNow());
    }
    if (m_phase==Phase::CONFIGURING && m_remoteMonitor) {
        auto w=m_window.lock();
        auto logical=Vector2D{static_cast<double>(m_viewport.width),static_cast<double>(m_viewport.height)}/m_viewport.scale;
        bool monitorReady=closeSize(m_remoteMonitor->m_pixelSize,{static_cast<double>(m_viewport.width),static_cast<double>(m_viewport.height)}) && std::abs(m_remoteMonitor->m_scale-m_viewport.scale)<.001;
        if (frameReady(monitorReady && matchesFrame(w,logical,m_viewport.scale))) {
            focusInput(inputWindow()); m_phase=Phase::ACTIVE;
        }
    }
    if (m_phase==Phase::RESTORING && m_originalMonitor) {
        if (frameReady(matchesFrame(m_window.lock(),m_restoreBox.size(),m_originalMonitor->m_scale)) && dialogsReady) { finishRestore(true); return; }
    }
    if (m_phase!=Phase::ACTIVE && std::chrono::steady_clock::now()>m_deadline) {
        if (m_phase==Phase::RESTORING) finishRestore(false);
        else restore("App or placeholder did not become ready");
    }
}

Json CDesktopLease::description(bool secrets) const {
    static const char* phases[]={"preparing","configuring","active","restoring","restored","closed","failed"};
    Json result={{"ok",true},{"v",1},{"phase",phases[static_cast<unsigned>(m_phase)]},{"active",!finished()},
        {"lease_id",m_id},{"ui_ready",uiReady()},{"input_ready",inputAllowed()&&m_seat.ready()},
        {"window_id",m_window?std::format("{:x}",m_window->metadata().stableID()):""},
        {"output",m_mobileOutput},{"reason",m_reason},{"width",m_viewport.width},{"height",m_viewport.height},{"scale",m_viewport.scale}};
    if (secrets) {
        result["ui_token"]=m_uiToken; result["input_token"]=m_inputToken; result["snapshot_path"]=m_snapshotPath;
        result["desktop_width"]=m_originalBox.w; result["desktop_height"]=m_originalBox.h;
        result["capture_handle"]=m_window?std::format("0x{:x}",reinterpret_cast<uintptr_t>(m_window.get())):"";
    }
    return result;
}
}
