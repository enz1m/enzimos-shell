#pragma once
#include <cmath>

namespace Enzui {
struct IconDropLease {
    bool held = false, armed = false, pending = false;
    double pressX = 0, pressY = 0, releaseX = 0, releaseY = 0, releasedAt = 0;

    void press(bool qualified, double x, double y) {
        held = true;
        pending = false;
        armed = qualified && std::isfinite(x) && std::isfinite(y);
        pressX = x;
        pressY = y;
    }
    void release(double now, double x, double y) {
        pending = held && armed && std::isfinite(now) && std::isfinite(x) && std::isfinite(y) &&
                  std::hypot(x - pressX, y - pressY) > 8;
        held = armed = false;
        releaseX = x;
        releaseY = y;
        releasedAt = now;
    }
    void cancel() { armed = pending = false; }
    bool take(bool qualified, double now, double x, double y) {
        const bool accepted = qualified && pending && !held && std::isfinite(now) &&
                              std::isfinite(x) && std::isfinite(y) && now >= releasedAt &&
                              now - releasedAt <= 1 && std::hypot(x - releaseX, y - releaseY) <= 2;
        pending = false;
        return accepted;
    }
};
}
