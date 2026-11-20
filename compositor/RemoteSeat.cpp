#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "RemoteSeat.hpp"
#include "src/Compositor.hpp"
#include <algorithm>
#include <cerrno>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <fcntl.h>
#include <format>
#include <linux/input-event-codes.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

namespace Enzremote {
namespace {
int keymapFile(xkb_keymap* map, uint32_t& size) {
    char* text = xkb_keymap_get_as_string(map, XKB_KEYMAP_FORMAT_TEXT_V1);
    if (!text) return -1;
    const size_t length = strlen(text) + 1;
    int fd = memfd_create("enzremote-seat-keymap", MFD_CLOEXEC | MFD_ALLOW_SEALING);
    size_t written = 0;
    while (fd >= 0 && written < length) {
        auto count = write(fd, text + written, length - written);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) { close(fd); fd = -1; break; }
        written += count;
    }
    free(text);
    if (fd >= 0 && (lseek(fd, 0, SEEK_SET) < 0 || fcntl(fd, F_ADD_SEALS, F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL) < 0)) { close(fd); fd = -1; }
    size = static_cast<uint32_t>(length);
    return fd;
}

bool modifier(uint32_t key) {
    switch (key) {
        case KEY_LEFTSHIFT: case KEY_RIGHTSHIFT: case KEY_LEFTCTRL: case KEY_RIGHTCTRL:
        case KEY_LEFTALT: case KEY_RIGHTALT: case KEY_LEFTMETA: case KEY_RIGHTMETA:
        case KEY_CAPSLOCK: case KEY_NUMLOCK: case KEY_SCROLLLOCK: return true;
        default: return false;
    }
}

bool decode(std::string_view text, std::vector<uint32_t>& result) {
    if (text.size() > 4096) return false;
    for (size_t i = 0; i < text.size();) {
        const auto first = static_cast<unsigned char>(text[i++]);
        uint32_t value = first;
        unsigned remaining = 0;
        uint32_t minimum = 0;
        if (first >= 0xF0 && first <= 0xF4) { value &= 7; remaining = 3; minimum = 0x10000; }
        else if (first >= 0xE0 && first <= 0xEF) { value &= 15; remaining = 2; minimum = 0x800; }
        else if (first >= 0xC2 && first <= 0xDF) { value &= 31; remaining = 1; minimum = 0x80; }
        else if (first >= 0x80) return false;
        while (remaining--) {
            if (i == text.size()) return false;
            auto next = static_cast<unsigned char>(text[i++]);
            if ((next & 0xC0) != 0x80) return false;
            value = (value << 6) | (next & 0x3F);
        }
        if (value < minimum || value > 0x10FFFF || (value >= 0xD800 && value <= 0xDFFF) || value < 32 || value == 127) return false;
        result.push_back(value);
    }
    return true;
}
}

CRemoteSeat::CRemoteSeat() : IWaylandProtocol(&wl_seat_interface, 9, "enzremote-seat") {
    m_context = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
    if (!m_context) return;
    xkb_rule_names names = {.rules = "evdev", .model = "pc105", .layout = "us", .variant = "", .options = ""};
    m_keymap = xkb_keymap_new_from_names(m_context, &names, XKB_KEYMAP_COMPILE_NO_FLAGS);
    if (!m_keymap) return;
    m_state = xkb_state_new(m_keymap);
    if (m_state) m_keymapFd = keymapFile(m_keymap, m_keymapSize);
}

CRemoteSeat::~CRemoteSeat() {
    clear();
    m_keyboards.clear(); m_pointers.clear(); m_seats.clear();
    if (m_keymapFd >= 0) close(m_keymapFd);
    if (m_state) xkb_state_unref(m_state);
    if (m_keymap) xkb_keymap_unref(m_keymap);
    if (m_context) xkb_context_unref(m_context);
}

bool CRemoteSeat::good() const { return m_keymapFd >= 0 && m_state; }
wl_client* CRemoteSeat::client() const { return m_focus ? m_focus->client() : nullptr; }
uint32_t CRemoteSeat::serial() const { return wl_display_next_serial(g_pCompositor->m_wlDisplay); }
uint32_t CRemoteSeat::timeMs() const {
    timespec now{}; clock_gettime(CLOCK_MONOTONIC, &now);
    return static_cast<uint32_t>(static_cast<uint64_t>(now.tv_sec) * 1000 + now.tv_nsec / 1000000);
}

bool CRemoteSeat::owns(wl_resource* resource) const {
    return std::ranges::any_of(m_seats, [resource](const auto& seat) { return seat->resource->resource() == resource; });
}

void CRemoteSeat::bindManager(wl_client* client_, void*, uint32_t version, uint32_t id) {
    const auto seat = makeShared<SSeat>();
    seat->client = client_;
    seat->resource = makeShared<CWlSeat>(client_, std::min(version, 9U), id);
    if (!seat->resource->resource()) { wl_client_post_no_memory(client_); return; }
    m_seats.push_back(seat);
    auto remove = [this, weak = WP<SSeat>(seat)](CWlSeat*) {
        if (auto value = weak.lock()) std::erase(m_seats, value);
    };
    seat->resource->setRelease(std::move(remove));
    seat->resource->setOnDestroy([this, weak = WP<SSeat>(seat)](CWlSeat*) {
        if (auto value = weak.lock()) std::erase(m_seats, value);
    });
    seat->resource->setGetPointer([this](CWlSeat* parent, uint32_t id_) {
        auto pointer = makeShared<SPointer>(); pointer->client = parent->client();
        pointer->resource = makeShared<CWlPointer>(parent->client(), parent->version(), id_);
        if (!pointer->resource->resource()) { parent->noMemory(); return; }
        m_pointers.push_back(pointer);
        pointer->resource->setRelease([this, weak = WP<SPointer>(pointer)](CWlPointer*) {
            if (auto value = weak.lock()) std::erase(m_pointers, value);
        });
        pointer->resource->setOnDestroy([this, weak = WP<SPointer>(pointer)](CWlPointer*) {
            if (auto value = weak.lock()) std::erase(m_pointers, value);
        });
        pointer->resource->setSetCursor([](CWlPointer*, uint32_t, wl_resource*, int32_t, int32_t) {});
        if (m_pointerFocus && pointer->client == client()) motion(m_pointerFocus.lock(), m_pointerLocal);
    });
    seat->resource->setGetKeyboard([this](CWlSeat* parent, uint32_t id_) {
        auto keyboard = makeShared<SKeyboard>(); keyboard->client = parent->client();
        keyboard->resource = makeShared<CWlKeyboard>(parent->client(), parent->version(), id_);
        if (!keyboard->resource->resource()) { parent->noMemory(); return; }
        m_keyboards.push_back(keyboard);
        keyboard->resource->setRelease([this, weak = WP<SKeyboard>(keyboard)](CWlKeyboard*) {
            if (auto value = weak.lock()) std::erase(m_keyboards, value);
        });
        keyboard->resource->setOnDestroy([this, weak = WP<SKeyboard>(keyboard)](CWlKeyboard*) {
            if (auto value = weak.lock()) std::erase(m_keyboards, value);
        });
        if (good()) keyboard->resource->sendKeymap(WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1, m_keymapFd, m_keymapSize);
        if (keyboard->resource->version() >= 4) keyboard->resource->sendRepeatInfo(25, 600);
        if (keyboard->client == client()) enterKeyboard(keyboard);
    });
    seat->resource->setGetTouch([](CWlSeat* resource, uint32_t) {
        resource->error(WL_SEAT_ERROR_MISSING_CAPABILITY, "Remote seat does not advertise native touch");
    });
    if (version >= 2) seat->resource->sendName("enzremote");
    const auto caps = good() && client_ == client() ? WL_SEAT_CAPABILITY_POINTER | WL_SEAT_CAPABILITY_KEYBOARD : 0;
    seat->resource->sendCapabilities(static_cast<wl_seat_capability>(caps));
}

void CRemoteSeat::enterKeyboard(SP<SKeyboard> keyboard) {
    if (!m_focus || keyboard->client != client()) return;
    wl_array keys; wl_array_init(&keys);
    for (uint32_t code = 0; code < m_keys.size(); ++code) if (m_keys[code]) {
        auto value = static_cast<uint32_t*>(wl_array_add(&keys, sizeof(uint32_t)));
        if (!value) { wl_array_release(&keys); return; }
        *value = code;
    }
    keyboard->resource->sendEnter(serial(), m_focus->getResource().get(), &keys);
    wl_array_release(&keys);
    keyboard->entered = m_focus;
    sendModifiers();
}

void CRemoteSeat::focus(SP<CWLSurfaceResource> surface) {
    if (surface == m_focus.lock()) return;
    if (!surface) { clear(); return; }
    if (client() != surface->client()) clear();
    for (auto& keyboard : m_keyboards) if (keyboard->entered) {
        keyboard->resource->sendLeave(serial(), keyboard->entered->getResource().get());
        keyboard->entered.reset();
    }
    m_focus = surface;
    for (auto& seat : m_seats) {
        const auto caps = seat->client == client() ? WL_SEAT_CAPABILITY_POINTER | WL_SEAT_CAPABILITY_KEYBOARD : 0;
        seat->resource->sendCapabilities(static_cast<wl_seat_capability>(caps));
    }
    for (auto& keyboard : m_keyboards) if (keyboard->client == client()) enterKeyboard(keyboard);
}

void CRemoteSeat::releaseInput() {
    if (good()) {
        for (uint32_t code = 0; code < m_keys.size(); ++code) if (m_keys[code]) key(code, false);
        xkb_state_unref(m_state); m_state = xkb_state_new(m_keymap);
        sendModifiers();
    }
    if (!m_buttons.empty()) { sendKey(KEY_ESC, true); sendKey(KEY_ESC, false); }
    while (!m_buttons.empty()) button(m_buttons.back(), false);
    m_keys.fill(false); m_buttons.clear();
}

void CRemoteSeat::clear() {
    releaseInput();
    for (auto& keyboard : m_keyboards) if (keyboard->entered) {
        keyboard->resource->sendLeave(serial(), keyboard->entered->getResource().get()); keyboard->entered.reset();
    }
    for (auto& pointer : m_pointers) if (pointer->entered) {
        pointer->resource->sendLeave(serial(), pointer->entered->getResource().get()); pointer->entered.reset(); pointerFrame(pointer);
    }
    m_focus.reset(); m_pointerFocus.reset();
    for (auto& seat : m_seats) seat->resource->sendCapabilities(static_cast<wl_seat_capability>(0));
}

bool CRemoteSeat::ready() const {
    if (!good() || !m_focus) return false;
    return std::ranges::any_of(m_keyboards, [this](const auto& value) { return value->client == client(); }) &&
           std::ranges::any_of(m_pointers, [this](const auto& value) { return value->client == client(); });
}

void CRemoteSeat::pointerFrame(SP<SPointer> pointer) {
    if (pointer->resource->version() >= 5) pointer->resource->sendFrame();
}

bool CRemoteSeat::motion(SP<CWLSurfaceResource> surface, const Vector2D& local) {
    if (!surface || surface->client() != client() || !std::isfinite(local.x) || !std::isfinite(local.y)) return false;
    m_pointerFocus = surface; m_pointerLocal = local;
    bool delivered = false;
    for (auto& pointer : m_pointers) if (pointer->client == client()) {
        if (pointer->entered.lock() != surface) {
            if (pointer->entered) pointer->resource->sendLeave(serial(), pointer->entered->getResource().get());
            pointer->resource->sendEnter(serial(), surface->getResource().get(), wl_fixed_from_double(local.x), wl_fixed_from_double(local.y));
            pointer->entered = surface;
        } else pointer->resource->sendMotion(timeMs(), wl_fixed_from_double(local.x), wl_fixed_from_double(local.y));
        pointerFrame(pointer); delivered = true;
    }
    return delivered;
}

bool CRemoteSeat::button(uint32_t code, bool pressed) {
    if (code < BTN_MOUSE || code > BTN_TASK) return false;
    auto found = std::ranges::find(m_buttons, code);
    if (pressed && found != m_buttons.end()) return true;
    if (!pressed && found == m_buttons.end()) return true;
    if (pressed) m_buttons.push_back(code); else std::erase(m_buttons, code);
    bool delivered = false;
    for (auto& pointer : m_pointers) if (pointer->entered && pointer->client == client()) {
        pointer->resource->sendButton(serial(), timeMs(), code, pressed ? WL_POINTER_BUTTON_STATE_PRESSED : WL_POINTER_BUTTON_STATE_RELEASED);
        pointerFrame(pointer); delivered = true;
    }
    return delivered;
}

bool CRemoteSeat::scroll(double dx, double dy, bool stop) {
    if (!std::isfinite(dx) || !std::isfinite(dy) || std::abs(dx) > 8192 || std::abs(dy) > 8192) return false;
    bool delivered = false;
    for (auto& pointer : m_pointers) if (pointer->entered && pointer->client == client()) {
        for (auto axis : {WL_POINTER_AXIS_VERTICAL_SCROLL, WL_POINTER_AXIS_HORIZONTAL_SCROLL}) {
            double value = axis == WL_POINTER_AXIS_VERTICAL_SCROLL ? dy : dx;
            if (value != 0) {
                pointer->resource->sendAxis(timeMs(), axis, wl_fixed_from_double(value));
                if (pointer->resource->version() >= 5) pointer->resource->sendAxisSource(WL_POINTER_AXIS_SOURCE_FINGER);
            }
            if (stop && pointer->resource->version() >= 5) pointer->resource->sendAxisStop(timeMs(), axis);
        }
        pointerFrame(pointer); delivered = true;
    }
    return delivered;
}

void CRemoteSeat::sendModifiers() {
    if (!m_state) return;
    for (auto& keyboard : m_keyboards) if (keyboard->entered && keyboard->client == client())
        keyboard->resource->sendModifiers(serial(), xkb_state_serialize_mods(m_state, XKB_STATE_MODS_DEPRESSED),
            xkb_state_serialize_mods(m_state, XKB_STATE_MODS_LATCHED), xkb_state_serialize_mods(m_state, XKB_STATE_MODS_LOCKED),
            xkb_state_serialize_layout(m_state, XKB_STATE_LAYOUT_EFFECTIVE));
}

void CRemoteSeat::sendKey(uint32_t code, bool pressed) {
    for (auto& keyboard : m_keyboards) if (keyboard->entered && keyboard->client == client())
        keyboard->resource->sendKey(serial(), timeMs(), code, pressed ? WL_KEYBOARD_KEY_STATE_PRESSED : WL_KEYBOARD_KEY_STATE_RELEASED);
}

bool CRemoteSeat::key(uint32_t code, bool pressed) {
    if (!good() || !m_focus || code >= m_keys.size()) return false;
    if (m_keys[code] == pressed) return true;
    m_keys[code] = pressed;
    xkb_state_update_key(m_state, code + 8, pressed ? XKB_KEY_DOWN : XKB_KEY_UP);
    sendKey(code, pressed); sendModifiers(); return true;
}

void CRemoteSeat::sendKeymap(int fd, uint32_t size) {
    for (auto& keyboard : m_keyboards) if (keyboard->client == client())
        keyboard->resource->sendKeymap(WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1, fd, size);
}

bool CRemoteSeat::text(std::string_view utf8) {
    if (!ready()) return false;
    std::vector<uint32_t> codepoints;
    if (!decode(utf8, codepoints)) return false;
    for (uint32_t code = 0; code < m_keys.size(); ++code) if (m_keys[code] && !modifier(code)) return false;
    std::vector<uint32_t> codes;
    for (uint32_t code = 16; code < 248; ++code) if (!modifier(code) && !m_keys[code]) codes.push_back(code);
    bool success = true;
    for (size_t start = 0; start < codepoints.size();) {
        size_t end = start;
        std::vector<uint32_t> symbols;
        while (end < codepoints.size()) {
            auto cp = codepoints[end];
            if (std::ranges::find(symbols, cp) == symbols.end()) {
                if (symbols.size() == 128) break;
                symbols.push_back(cp);
            }
            ++end;
        }
        std::string keycodes = "xkb_keymap { xkb_keycodes \"remote\" { minimum=8; maximum=255; ";
        std::string definitions = "xkb_symbols \"remote\" { ";
        for (size_t i = 0; i < symbols.size(); ++i) {
            const auto name = std::format("K{:03}", i);
            const auto value = std::format("U{:08X}", symbols[i]);
            keycodes += "<" + name + ">=" + std::to_string(codes[i] + 8) + ";";
            definitions += "key <" + name + "> { type=\"ONE_LEVEL\", [ " + value + " ] };";
        }
        auto source = keycodes + "}; xkb_types \"remote\" { type \"ONE_LEVEL\" { modifiers=None; map[None]=Level1; }; }; xkb_compatibility \"remote\" {}; " + definitions + "}; };";
        auto map = xkb_keymap_new_from_string(m_context, source.c_str(), XKB_KEYMAP_FORMAT_TEXT_V1, XKB_KEYMAP_COMPILE_NO_FLAGS);
        if (!map) { success = false; break; }
        uint32_t size = 0; int fd = keymapFile(map, size); xkb_keymap_unref(map);
        if (fd < 0) { success = false; break; }
        sendKeymap(fd, size); close(fd);
        for (auto& keyboard : m_keyboards) if (keyboard->client == client()) keyboard->resource->sendModifiers(serial(), 0, 0, 0, 0);
        for (size_t i = start; i < end; ++i) {
            auto index = std::ranges::find(symbols, codepoints[i]) - symbols.begin();
            sendKey(codes[index], true); sendKey(codes[index], false);
        }
        start = end;
    }
    sendKeymap(m_keymapFd, m_keymapSize); sendModifiers();
    return success;
}

}
