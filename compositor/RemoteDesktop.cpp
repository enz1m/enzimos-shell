#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "RemoteDesktop.hpp"
#include "../vendor/json/single_include/nlohmann/json.hpp"
#include "src/Compositor.hpp"
#include "src/desktop/state/WindowState.hpp"
#include "src/desktop/state/FocusState.hpp"
#include "src/event/EventBus.hpp"
#include "src/output/Monitor.hpp"
#include "src/state/MonitorState.hpp"
#include "src/render/Renderer.hpp"
#include "src/protocols/XDGShell.hpp"
#include <cerrno>
#include <cmath>
#include <cstring>
#include <format>
#include <linux/input-event-codes.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

namespace Enzremote {
using Json = nlohmann::json;

CRemoteDesktop::CRemoteDesktop() {
    m_seat = std::make_unique<CRemoteSeat>();
    if (!m_seat->good()) { LOG(Log::ERR, "[enzremote] Remote seat keymap setup failed"); return; }
    m_path = g_pCompositor->m_instancePath + "/.enzremote.sock";
    sockaddr_un address{}; address.sun_family = AF_UNIX;
    if (m_path.size() >= sizeof(address.sun_path)) { LOG(Log::ERR, "[enzremote] Control socket path is too long"); return; }
    memcpy(address.sun_path, m_path.c_str(), m_path.size() + 1);
    m_listener = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC | SOCK_NONBLOCK, 0);
    if (m_listener < 0) return;
    if (bind(m_listener, reinterpret_cast<sockaddr*>(&address), sizeof(address)) < 0 || chmod(m_path.c_str(), 0600) < 0 || listen(m_listener, 8) < 0) {
        LOG(Log::ERR, "[enzremote] Cannot create private control socket: {}", strerror(errno));
        close(m_listener); m_listener = -1; return;
    }
    m_listenerSource = wl_event_loop_add_fd(g_pCompositor->m_wlEventLoop, m_listener, WL_EVENT_READABLE, acceptEvent, this);
    m_timer = wl_event_loop_add_timer(g_pCompositor->m_wlEventLoop, timerEvent, this);
    if (m_timer) wl_event_source_timer_update(m_timer, 16);
    m_opened=Event::bus()->m_events.window.openLate.listen([this](PHLWINDOW window) {
        if (!m_lease) return;
        try { m_lease->windowMapped(window); }
        catch (const std::exception& error) {
            LOG(Log::ERR,"[enzremote] Transient attachment: {}",error.what());
            try { m_lease->restore("Transient window attachment failed"); } catch (...) {}
        }
    });
    LOG(Log::INFO, "[enzremote] Shared desktop control ready");
}

CRemoteDesktop::~CRemoteDesktop() {
    m_opened.reset();
    if (m_timer) wl_event_source_remove(m_timer);
    detach();
    while (!m_connections.empty()) disconnect(m_connections.begin()->first);
    if (m_listenerSource) wl_event_source_remove(m_listenerSource);
    if (m_listener >= 0) { close(m_listener); unlink(m_path.c_str()); }
    m_lease.reset();
    m_seat.reset();
}

bool CRemoteDesktop::ownsSeat(wl_resource* resource) const { return m_seat && m_seat->owns(resource); }
bool CRemoteDesktop::ownsSurface(SP<CWLSurfaceResource> surface) const {
    auto window = m_window ? m_window.lock() : (m_lease ? m_lease->window() : nullptr);
    if (!window || !surface) return false;
    auto root = window->backend().surface();
    if (!root) return false;
    if (root->findFirstPreorder([&](auto candidate) { return candidate == surface; })) return true;
    if (m_lease) for (const auto& candidate : Desktop::windowState()->windows()) {
        if (!candidate || !m_lease->ownsWindow(candidate)) continue;
        auto other=candidate->backend().surface();
        if (other && other->findFirstPreorder([&](auto part) { return part==surface; })) return true;
    }
    for (const auto& entry : m_popups) if (entry->popup && entry->popup->m_surface && entry->popup->m_surface->m_surface) {
        if (entry->popup->m_surface->m_surface->findFirstPreorder([&](auto candidate) { return candidate == surface; })) return true;
    }
    return false;
}

bool CRemoteDesktop::blocksLocalSurface(SP<CWLSurfaceResource> surface) const {
    return m_lease && !m_lease->finished() && m_lease->phase() != CDesktopLease::Phase::PREPARING && ownsSurface(surface);
}

int CRemoteDesktop::timerEvent(void* data) {
    auto self = static_cast<CRemoteDesktop*>(data);
    try {
        if (self->m_lease) {
            self->m_lease->tick();
            if (self->m_lease->finished() && self->m_window) {
                self->m_seat->clear(); self->m_window.reset(); self->m_unmapped.reset(); self->m_popups.clear();
                for (auto& [fd, connection] : self->m_connections) connection->inputBound = false;
            }
        }
    } catch (const std::exception& error) {
        LOG(Log::ERR, "[enzremote] Lease transition: {}", error.what());
    }
    wl_event_source_timer_update(self->m_timer, 16);
    return 0;
}

bool CRemoteDesktop::popupGrab(wl_resource* seat, SP<CXDGPopupResource> popup) {
    if (!ownsSeat(seat)) return false;
    if (!m_window || !popup || !popup->m_parent || !popup->m_parent->m_surface || !ownsSurface(popup->m_parent->m_surface.lock())) return true;
    std::erase_if(m_popups, [](const auto& entry) { return entry->popup.expired(); });
    if (m_popups.size() >= 32) { popup->done(); return true; }
    auto entry = std::make_unique<SPopup>(); entry->popup = popup;
    entry->destroyed = popup->m_events.destroy.listen([this]() {
        if (m_lease && !m_lease->inputAllowed()) return;
        auto window=m_lease ? m_lease->inputWindow() : m_window.lock();
        if (window && window->mapped()) m_seat->focus(window->backend().surface());
    });
    m_popups.push_back(std::move(entry));
    if (popup->m_surface && popup->m_surface->m_surface) m_seat->focus(popup->m_surface->m_surface.lock());
    return true;
}

int CRemoteDesktop::acceptEvent(int, uint32_t, void* data) {
    static_cast<CRemoteDesktop*>(data)->acceptClients(); return 0;
}

void CRemoteDesktop::acceptClients() {
    for (unsigned accepted = 0; accepted < 8; ++accepted) {
        int fd = accept4(m_listener, nullptr, nullptr, SOCK_CLOEXEC | SOCK_NONBLOCK);
        if (fd < 0) return;
        ucred peer{}; socklen_t size = sizeof(peer);
        if (getsockopt(fd, SOL_SOCKET, SO_PEERCRED, &peer, &size) < 0 || peer.uid != getuid() || m_connections.size() >= 16) { close(fd); continue; }
        auto connection = std::make_unique<SConnection>(); connection->owner = this; connection->fd = fd; connection->pid = peer.pid;
        connection->source = wl_event_loop_add_fd(g_pCompositor->m_wlEventLoop, fd, WL_EVENT_READABLE, clientEvent, connection.get());
        if (!connection->source) { close(fd); continue; }
        m_connections.emplace(fd, std::move(connection));
    }
}

int CRemoteDesktop::clientEvent(int fd, uint32_t mask, void* data) {
    auto connection = static_cast<SConnection*>(data); auto owner = connection->owner;
    if ((mask & (WL_EVENT_HANGUP | WL_EVENT_ERROR)) || ((mask & WL_EVENT_READABLE) && !owner->readClient(*connection)) ||
        ((mask & WL_EVENT_WRITABLE) && !owner->writeClient(*connection))) {
        owner->disconnect(fd); return 0;
    }
    return 0;
}

bool CRemoteDesktop::readClient(SConnection& connection) {
    char buffer[8192];
    unsigned requests = 0;
    for (unsigned batch = 0; batch < 8; ++batch) {
        auto count = recv(connection.fd, buffer, sizeof(buffer), 0);
        if (count == 0) return false;
        if (count < 0) { if (errno == EINTR) continue; return errno == EAGAIN || errno == EWOULDBLOCK; }
        connection.input.append(buffer, count);
        if (connection.input.size() > 65536) return false;
        for (;;) {
            auto end = connection.input.find('\n');
            if (end == std::string::npos) break;
            if (++requests > 128) return false;
            auto line = connection.input.substr(0, end); connection.input.erase(0, end + 1);
            Json result;
            try {
                auto request = Json::parse(line);
                if (!request.is_object() || request.value("v", 0) != 1) throw std::runtime_error("Unsupported request version");
                result = handle(connection, request);
            } catch (const std::exception& error) {
                result = {{"ok", false}, {"error", std::string(error.what()).substr(0, 240)}};
            }
            connection.output += result.dump(-1, ' ', false, Json::error_handler_t::replace) + "\n";
            if (connection.output.size() > 262144 || !writeClient(connection)) return false;
        }
    }
    return true;
}

bool CRemoteDesktop::writeClient(SConnection& connection) {
    while (!connection.output.empty()) {
        auto count = send(connection.fd, connection.output.data(), connection.output.size(), MSG_NOSIGNAL);
        if (count < 0) {
            if (errno == EINTR) continue;
            if (errno != EAGAIN && errno != EWOULDBLOCK) return false;
            wl_event_source_fd_update(connection.source, WL_EVENT_READABLE | WL_EVENT_WRITABLE); return true;
        }
        if (!count) return false;
        connection.output.erase(0, count);
    }
    wl_event_source_fd_update(connection.source, WL_EVENT_READABLE); return true;
}

void CRemoteDesktop::disconnect(int fd) {
    if (fd == m_ui) {
        m_ui = -1;
        try { if (m_lease) m_lease->restore("Desktop placeholder closed"); } catch (...) {}
    }
    if (fd == m_controller) detach();
    auto found = m_connections.find(fd);
    if (found == m_connections.end()) return;
    if (found->second->inputBound && m_seat) {
        if (m_lease && !m_lease->finished()) m_seat->releaseInput();
        else m_seat->clear();
    }
    wl_event_source_remove(found->second->source); close(fd); m_connections.erase(found);
}

Json CRemoteDesktop::list() const {
    Json windows = Json::array();
    for (const auto& window : Desktop::windowState()->windows()) {
        if (!window || !window->mapped() || window->backend().isX11()) continue;
        if (window->metadata().appID().starts_with("dev.enzui.")) continue;
        auto position = window->position(Desktop::View::IGeometric::GEOMETRIC_GOAL);
        auto size = window->size(Desktop::View::IGeometric::GEOMETRIC_GOAL);
        auto parent=window->backend().parent();
        windows.push_back({{"window_id", std::format("{:x}", window->metadata().stableID())}, {"pid", window->backend().pid()},
            {"parent_window_id",parent ? std::format("{:x}",parent->metadata().stableID()) : ""},
            {"title", window->metadata().title().substr(0, 512)}, {"class", window->metadata().appID().substr(0, 128)},
            {"x", position.x}, {"y", position.y}, {"width", size.x}, {"height", size.y}, {"floating", window->isFloating()}});
        if (windows.size() == 256) break;
    }
    return windows;
}

Json CRemoteDesktop::status() const {
    if (m_lease) return m_lease->description();
    return {{"ok", true}, {"v", 1}, {"active", !!m_window}, {"input_ready", m_seat && m_seat->ready()},
        {"window_id", m_window ? std::format("{:x}", m_window->metadata().stableID()) : ""}};
}

static SViewport viewport(const Json& value) {
    SViewport result{value.at("width").get<uint32_t>(), value.at("height").get<uint32_t>(), value.value("refresh",60u), value.at("scale").get<double>()};
    if (result.width<240 || result.height<240 || result.width>4096 || result.height>4096 || result.refresh<15 || result.refresh>120 ||
        !std::isfinite(result.scale) || result.scale<.5 || result.scale>4 || result.width/result.scale<160 || result.height/result.scale<160)
        throw std::runtime_error("Unsupported mobile viewport");
    return result;
}

Json CRemoteDesktop::handle(SConnection& connection, const Json& request) {
    const auto operation = request.at("op").get<std::string>();
    if (operation == "ping") return {{"ok",true},{"v",1},{"remote_seat",m_seat->good()},{"handoff",true}};
    if (operation == "list") return {{"ok",true},{"windows",list()}};
    if (operation == "repaint") {
        const auto output=request.at("output").get<std::string>();
        if (output.empty() || output.size()>80) throw std::runtime_error("Invalid repaint output");
        auto monitor=State::monitorState()->query().name(output).run();
        if (!monitor || !monitor->enabled()) throw std::runtime_error("Repaint output is unavailable");
        g_pHyprRenderer->damageMonitor(monitor);
        monitor->scheduleFrame(Aquamarine::IOutput::AQ_SCHEDULE_CLIENT_UNKNOWN);
        return {{"ok",true}};
    }
    if (operation == "status") return status();
    if (operation == "prepare" || operation == "probe_attach") {
        if ((m_lease && !m_lease->finished()) || (!m_lease && m_controller>=0)) throw std::runtime_error("A window is already controlled");
        if (operation=="probe_attach") {
            const auto allowed=getenv("ENZREMOTE_ALLOW_INPUT_PROBE");
            if (!allowed || std::string_view(allowed)!="1") throw std::runtime_error("Input probe is disabled");
        }
        m_lease.reset(); detach(); m_ui=-1;
        const auto id=request.at("window_id").get<std::string>();
        for (const auto& window : Desktop::windowState()->windows()) {
            if (!window || !window->mapped() || window->backend().isX11() || std::format("{:x}",window->metadata().stableID())!=id) continue;
            if (window->metadata().appID().starts_with("dev.enzui.")) throw std::runtime_error("Shell surfaces are not app sources");
            m_window=window; m_controller=connection.fd;
            m_unmapped=window->m_events.unmap.listen([this]() { if (m_lease) m_lease->windowClosed(); else detach(); });
            if (operation=="prepare") {
                unsigned handles=0;
                const auto handle=static_cast<uint32_t>(reinterpret_cast<uintptr_t>(window.get()));
                for (const auto& candidate : Desktop::windowState()->windows())
                    if (candidate && candidate->mapped() && static_cast<uint32_t>(reinterpret_cast<uintptr_t>(candidate.get()))==handle) ++handles;
                if (handles!=1) throw std::runtime_error("Ambiguous window capture handle");
                m_lease=std::make_unique<CDesktopLease>(window,viewport(request.at("viewport")),*m_seat);
                m_lease->prepare(); return m_lease->description(true);
            }
            window->backend().setActive(true); m_seat->focus(window->backend().surface()); return status();
        }
        throw std::runtime_error("Window is no longer available");
    }
    if (operation=="ui_attach" && m_lease && m_lease->acceptsUI(request.at("token").get<std::string>())) {
        m_lease->bindUI(connection.pid); m_ui=connection.fd;
        auto result=m_lease->description(true); result.erase("input_token"); return result;
    }
    if (operation=="input_bind" && m_lease && m_lease->acceptsInput(request.at("token").get<std::string>())) {
        for (const auto& [fd, other] : m_connections) if (other->inputBound) throw std::runtime_error("Input is already connected");
        connection.inputBound=true;
        m_lease->focusInput(m_lease->inputWindow());
        return status();
    }
    if (operation=="content_size" && m_lease && m_lease->acceptsInput(request.at("token").get<std::string>())) {
        auto width=request.at("width").get<uint32_t>(); auto height=request.at("height").get<uint32_t>(); auto scale=request.at("scale").get<double>();
        m_lease->contentSize(width,height,scale);
        return {{"ok",true},{"width",width},{"height",height},{"scale",scale}};
    }
    if (operation=="restore" && m_lease && (connection.fd==m_controller || connection.fd==m_ui)) {
        m_lease->restore(connection.fd==m_ui?"Local takeover":"Remote released"); return status();
    }
    if (operation=="input" && connection.inputBound) return {{"ok",input(request.at("event"))}};
    if (connection.fd!=m_controller) throw std::runtime_error("Connection does not own the window");
    if (operation=="activate" && m_lease) { m_lease->activate(); return status(); }
    if (operation=="resize" && m_lease) { m_lease->resize(viewport(request.at("viewport"))); return status(); }
    if (operation=="detach") { detach(); return {{"ok",true}}; }
    if (operation=="input") return {{"ok",input(request.at("event"))}};
    throw std::runtime_error("Unsupported operation");
}

void CRemoteDesktop::detach() {
    try { if (m_lease) m_lease->restore("Controller disconnected"); } catch (const std::exception& error) { LOG(Log::ERR,"[enzremote] Restore: {}",error.what()); }
    for (auto& [fd, connection] : m_connections) connection->inputBound = false;
    m_unmapped.reset();
    m_popups.clear();
    if (m_seat) m_seat->clear();
    if (m_window && Desktop::focusState()->window() != m_window.lock()) m_window->backend().setActive(false);
    m_window.reset(); m_pointerWindow.reset(); m_controller = -1;
}

bool CRemoteDesktop::input(const Json& event) {
    if (m_lease && m_lease->inputAllowed() && !m_seat->ready()) m_lease->focusInput(m_lease->inputWindow());
    if (!m_window || !m_seat->ready() || (m_lease && !m_lease->inputAllowed())) return false;
    const auto type = event.at("type").get<std::string>();
    if (type == "key") return m_seat->key(event.at("key").get<uint32_t>(), event.at("pressed").get<bool>());
    if (type == "text") return m_seat->text(event.at("text").get<std::string>());
    if (type == "button") {
        if (m_lease && event.at("pressed").get<bool>()) m_lease->focusInput(m_pointerWindow.lock());
        return m_seat->button(event.at("button").get<uint32_t>(), event.at("pressed").get<bool>());
    }
    auto position = Vector2D{event.at("x").get<double>(), event.at("y").get<double>()};
    const auto scale = m_window->m_monitor ? m_window->m_monitor->m_scale : 1.0;
    position /= scale;
    auto size = m_window->size(Desktop::View::IGeometric::GEOMETRIC_GOAL);
    if (!std::isfinite(position.x) || !std::isfinite(position.y) || position.x < 0 || position.y < 0 || position.x >= size.x || position.y >= size.y) return false;
    auto root = m_window->backend().surface();
    if (!root) return false;
    auto [surface, local] = root->at(position, true);
    auto targetWindow=m_window.lock();
    const auto rootPosition=m_window->position(Desktop::View::IGeometric::GEOMETRIC_GOAL);
    if (m_lease) {
        const auto windows=m_lease->inputWindows();
        for (auto it=windows.rbegin(); it!=windows.rend(); ++it) {
            auto candidate=*it;
            const auto offset=candidate->position(Desktop::View::IGeometric::GEOMETRIC_GOAL)-rootPosition;
            auto hit=candidate->backend().surface()->at(position-offset,true);
            if (hit.first) { surface=hit.first; local=hit.second; targetWindow=candidate; break; }
        }
    }
    for (auto it = m_popups.rbegin(); it != m_popups.rend(); ++it) {
        const auto popup = (*it)->popup.lock();
        if (!popup || !popup->m_surface || !popup->m_surface->m_surface || !popup->m_surface->m_surface->m_mapped) continue;
        auto offset = popup->inputOffset();
        auto parent=popup->m_parent.lock();
        for (unsigned depth=0; parent && parent->m_popup && depth<32; ++depth) parent=parent->m_popup->m_parent.lock();
        auto parentWindow=parent && parent->m_toplevel ? parent->m_toplevel->m_window.lock() : nullptr;
        if (parentWindow) offset+=parentWindow->position(Desktop::View::IGeometric::GEOMETRIC_GOAL)-rootPosition;
        auto hit = popup->m_surface->m_surface->at(position - offset, true);
        if (hit.first) { surface = hit.first; local = hit.second; if (parentWindow) targetWindow=parentWindow; break; }
    }
    if (!surface || !m_seat->motion(surface, local)) return false;
    m_pointerWindow=targetWindow;
    if (type == "pointer") {
        if (event.value("click", false)) {
            if (m_lease) m_lease->focusInput(targetWindow);
            return m_seat->button(BTN_LEFT, true) && m_seat->button(BTN_LEFT, false);
        }
        return true;
    }
    if (type == "scroll_pixels") return m_seat->scroll(event.at("dx").get<double>() / scale, event.at("dy").get<double>() / scale, event.value("stop", false));
    return false;
}

}
