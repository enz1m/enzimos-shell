#pragma once
#include <algorithm>
#include <cmath>

namespace Enzui {
inline double searchDimProgress(double from, double goal, double seconds) {
    const auto t = std::clamp(seconds / 0.100, 0.0, 1.0);
    if (t == 1) return goal;
    const auto remaining = 1 - t;
    return from + (goal - from) * (1 - remaining * remaining * remaining);
}
inline double railOffset(double width, double frame, double progress) {
    return width - frame - std::round((width - frame) * std::clamp(progress, 0.0, 1.0));
}
inline double sceneTravelSeconds(double from, double goal, double rate = 100.0) {
    const auto factor = std::isfinite(rate) && rate >= 25 && rate <= 200 ? 100.0 / rate : 1.0;
    return std::max(0.060, (goal > from ? 0.180 : 0.140) * std::abs(goal - from)) * factor;
}
inline double iconTravelSeconds(double from, double goal, double rate = 100.0) {
    const auto factor = std::isfinite(rate) && rate >= 25 && rate <= 200 ? 100.0 / rate : 1.0;
    return std::max(0.090, (goal > from ? 0.300 : 0.260) * std::abs(goal - from)) * factor;
}
inline double iconProgress(double from, double goal, double seconds, double rate = 100.0) {
    const auto t = std::clamp(seconds / iconTravelSeconds(from, goal, rate), 0.0, 1.0);
    const auto eased = goal > from ? 1 - std::pow(1 - t, 3) : t * t * (3 - 2 * t);
    return from + (goal - from) * eased;
}
inline double pocketTravelSeconds(double from, double goal, double rate = 100.0) {
    const auto distance = std::min(1.0, std::abs(goal - from) / 0.36);
    return goal > from ? sceneTravelSeconds(0, distance, rate)
                       : sceneTravelSeconds(distance, 0, rate);
}
inline double pocketProgress(double from, double goal, double seconds, double rate = 100.0) {
    const auto t = std::clamp(seconds / pocketTravelSeconds(from, goal, rate), 0.0, 1.0);
    if (t == 1.0)
        return goal;
    const auto remaining = 1.0 - t;
    return from + (goal - from) * (1.0 - remaining * remaining * remaining);
}
inline double restoreTravelSeconds(double from, double rate = 100.0) {
    const auto factor = std::isfinite(rate) && rate >= 25 && rate <= 200 ? 100.0 / rate : 1.0;
    return std::max(0.060, 0.240 * (1.0 - std::clamp(from, 0.0, 1.0))) * factor;
}
inline double restoreProgress(double from, double seconds, double rate = 100.0) {
    const auto t = std::clamp(seconds / restoreTravelSeconds(from, rate), 0.0, 1.0);
    const auto remaining = 1.0 - t;
    return from + (1.0 - from) * (1.0 - remaining * remaining * remaining);
}
inline double sceneProgress(double from, double goal, double seconds, double rate = 100.0) {
    const auto t = std::clamp(seconds / sceneTravelSeconds(from, goal, rate), 0.0, 1.0);
    const auto remaining = 1.0 - t;
    return from + (goal - from) * (1.0 - remaining * remaining * remaining);
}
}
