#pragma once
#include <array>
#include <cmath>
#include <optional>

namespace Enzui {
inline double anchoredY(double top, double targetHeight, double committedHeight, bool bottom) {
    return bottom ? top + targetHeight - committedHeight : top;
}
// Resize staging paints only the overlap of the window box and committed buffer.
inline double paintedHeight(double committedHeight, double windowHeight) {
    return committedHeight < windowHeight ? committedHeight : windowHeight;
}
// Shell popup canvases keep up to 36 logical pixels for shadows beyond the
// output; the bar's full-width surface remains strictly inside it.
inline bool panelAnchorFits(double x, double y, double w, double h, double outputW, double outputH, bool bar) {
    for (double n : {x, y, w, h, outputW, outputH})
        if (!std::isfinite(n)) return false;
    const double padding = bar ? 0. : 36.;
    return outputW > 0 && outputH > 0 && w >= 80 && h >= (bar ? 32 : 80) &&
           (!bar || (x == 0 && w == outputW && h <= 72)) &&
           x >= -padding && y >= -padding && x + w <= outputW + padding && y + h <= outputH + padding;
}
enum class PlacementState { Pending, Ready, Failed };
inline PlacementState placementState(bool aligned, bool presented, bool expired) {
    if (aligned)
        return PlacementState::Ready;
    return presented || expired ? PlacementState::Failed : PlacementState::Pending;
}
struct ContentGeometry {
    double x, y, width, height, ackWidth, ackHeight, extentWidth, extentHeight;
    bool operator==(const ContentGeometry &) const = default;
    bool matches(double w, double h) const {
        for (double n : {x, y, width, height, ackWidth, ackHeight, extentWidth, extentHeight})
            if (!std::isfinite(n))
                return false;
        return w > 0 && h > 0 && width == w && height == h && ackWidth == w && ackHeight == h &&
               x >= 0 && y >= 0 && x + width <= extentWidth && y + height <= extentHeight;
    }
};
// Surface/texture identity is checked separately; metadata-only commits cannot
// replace the geometry qualified by an actual buffer attachment.
class CommittedGeometry {
    std::optional<ContentGeometry> attached;

  public:
    void observe(ContentGeometry value, bool bufferAttached) {
        if (bufferAttached)
            attached = value;
    }
    bool matches(ContentGeometry current, double w, double h) const {
        return attached && *attached == current && current.matches(w, h);
    }
};
}
