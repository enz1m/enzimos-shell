#pragma once
#include "RemoteSeat.hpp"
#include "DesktopLease.hpp"
#include "src/desktop/view/window/Window.hpp"
#include <memory>
#include <unordered_map>
#include "../vendor/json/single_include/nlohmann/json_fwd.hpp"

class CXDGPopupResource;

namespace Enzremote {

class CRemoteDesktop {
  public:
    CRemoteDesktop();
    ~CRemoteDesktop();
    bool ownsSeat(wl_resource*) const;
    bool ownsSurface(SP<CWLSurfaceResource>) const;
    bool blocksLocalSurface(SP<CWLSurfaceResource>) const;
    bool popupGrab(wl_resource*, SP<CXDGPopupResource>);

  private:
    struct SConnection {
        CRemoteDesktop* owner = nullptr;
        int fd = -1;
        int pid = -1;
        bool inputBound = false;
        wl_event_source* source = nullptr;
        std::string input;
        std::string output;
    };
    std::unique_ptr<CRemoteSeat> m_seat;
    std::unique_ptr<CDesktopLease> m_lease;
    wl_event_source* m_timer = nullptr;
    int m_ui = -1;
    int m_listener = -1;
    wl_event_source* m_listenerSource = nullptr;
    std::string m_path;
    std::unordered_map<int, std::unique_ptr<SConnection>> m_connections;
    int m_controller = -1;
    PHLWINDOWREF m_window;
    CHyprSignalListener m_unmapped;
    CHyprSignalListener m_opened;
    PHLWINDOWREF m_pointerWindow;
    struct SPopup { WP<CXDGPopupResource> popup; CHyprSignalListener destroyed; };
    std::vector<std::unique_ptr<SPopup>> m_popups;

    static int timerEvent(void*);
    static int acceptEvent(int, uint32_t, void*);
    static int clientEvent(int, uint32_t, void*);
    void acceptClients();
    bool readClient(SConnection&);
    bool writeClient(SConnection&);
    void disconnect(int);
    nlohmann::json handle(SConnection&, const nlohmann::json&);
    nlohmann::json status() const;
    nlohmann::json list() const;
    void detach();
    bool input(const nlohmann::json&);
};

inline std::unique_ptr<CRemoteDesktop> desktop;
}
