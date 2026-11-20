#pragma once

#include "src/protocols/WaylandProtocol.hpp"
#include "src/protocols/core/Compositor.hpp"
#include "wayland.hpp"
#include <array>
#include <string_view>
#include <vector>
#include <xkbcommon/xkbcommon.h>

namespace Enzremote {

class CRemoteSeat final : public IWaylandProtocol {
  public:
    CRemoteSeat();
    ~CRemoteSeat() override;
    void bindManager(wl_client*, void*, uint32_t, uint32_t) override;

    bool good() const;
    bool owns(wl_resource*) const;
    void focus(SP<CWLSurfaceResource> surface);
    void releaseInput();
    void clear();
    bool ready() const;
    bool motion(SP<CWLSurfaceResource> surface, const Vector2D& local);
    bool button(uint32_t code, bool pressed);
    bool scroll(double dx, double dy, bool stop);
    bool key(uint32_t code, bool pressed);
    bool text(std::string_view utf8);

  private:
    struct SSeat { SP<CWlSeat> resource; wl_client* client = nullptr; };
    struct SPointer { SP<CWlPointer> resource; wl_client* client = nullptr; WP<CWLSurfaceResource> entered; };
    struct SKeyboard { SP<CWlKeyboard> resource; wl_client* client = nullptr; WP<CWLSurfaceResource> entered; };

    std::vector<SP<SSeat>> m_seats;
    std::vector<SP<SPointer>> m_pointers;
    std::vector<SP<SKeyboard>> m_keyboards;
    WP<CWLSurfaceResource> m_focus;
    WP<CWLSurfaceResource> m_pointerFocus;
    Vector2D m_pointerLocal;
    std::array<bool, 768> m_keys{};
    std::vector<uint32_t> m_buttons;
    xkb_context* m_context = nullptr;
    xkb_keymap* m_keymap = nullptr;
    xkb_state* m_state = nullptr;
    int m_keymapFd = -1;
    uint32_t m_keymapSize = 0;

    uint32_t serial() const;
    uint32_t timeMs() const;
    wl_client* client() const;
    void enterKeyboard(SP<SKeyboard>);
    void sendModifiers();
    void sendKeymap(int fd, uint32_t size);
    void sendKey(uint32_t code, bool pressed);
    void pointerFrame(SP<SPointer>);
};

}
