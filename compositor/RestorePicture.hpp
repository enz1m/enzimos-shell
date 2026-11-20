#pragma once
#include <algorithm>
#include <cmath>
#include <optional>

namespace Enzui {
struct PictureCrop {
    double right, bottom;
};
inline std::optional<PictureCrop> coverCrop(double sourceWidth, double sourceHeight,
                                           double width, double height) {
    for (const auto value : {sourceWidth, sourceHeight, width, height})
        if (!std::isfinite(value) || value <= 0)
            return std::nullopt;
    const auto scale = std::max(width / sourceWidth, height / sourceHeight);
    return PictureCrop{width / (sourceWidth * scale), height / (sourceHeight * scale)};
}
}
