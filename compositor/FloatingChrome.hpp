#pragma once
#include <algorithm>
#include <cmath>
#include <vector>

namespace Enzui {
inline int protectedContentPadding(bool floating, bool restoring) {
    return floating && !restoring ? 0 : 6;
}
inline bool validPaneInset(bool floating, double inset) {
    return std::isfinite(inset) && inset == std::floor(inset) && inset >= 6 && inset <= 10 &&
           (floating || inset == 6);
}
struct InputSpan {
    int x, y, width, height;
};
inline bool capsuleContains(double width, double height, double x, double y) {
    if (!std::isfinite(width) || !std::isfinite(height) || !std::isfinite(x) || !std::isfinite(y) ||
        width <= 0 || height <= 0 || x < 0 || y < 0 || x >= width || y >= height)
        return false;
    const double radius = std::min(width, height) / 2;
    const double dx = std::max({radius - x, x - (width - radius), 0.0});
    const double dy = std::max({radius - y, y - (height - radius), 0.0});
    return dx * dx + dy * dy <= radius * radius;
}
inline std::vector<InputSpan> capsuleInput(int width, int height) {
    std::vector<InputSpan> result;
    if (width < 1 || width > 256 || height < 1 || height > 16384)
        return result;
    const double radius = std::min(width, height) / 2.0;
    for (int y = 0; y < height; ++y) {
        const double distance = std::max({radius - y - .5, y + .5 - (height - radius), 0.0});
        const double inset = radius - std::sqrt(std::max(0.0, radius * radius - distance * distance));
        const int x = std::max(0, static_cast<int>(std::ceil(inset - .5)));
        const int span = width - x * 2;
        if (span <= 0)
            continue;
        if (!result.empty() && result.back().x == x && result.back().width == span &&
            result.back().y + result.back().height == y)
            ++result.back().height;
        else
            result.push_back({x, y, span, 1});
    }
    return result;
}
}
