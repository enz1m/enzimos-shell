#include "../../compositor/SceneTiming.hpp"
#include <cassert>
#include <iostream>
#include <limits>

int main() {
    unsigned checks = 0;
    auto check = [&](bool value) { assert(value); ++checks; };
    for (auto from : {0.0, 0.25, 0.75, 1.0}) {
        for (auto goal : {0.0, 1.0}) {
            check(Enzui::searchDimProgress(from, goal, 0) == from);
            check(Enzui::searchDimProgress(from, goal, .100) == goal);
            auto previous = from;
            for (int step = 1; step <= 10; ++step) {
                const auto next = Enzui::searchDimProgress(from, goal, .010 * step);
                check(next >= std::min(from, goal) && next <= std::max(from, goal));
                check(goal >= from ? next >= previous : next <= previous);
                check(Enzui::searchDimProgress(next, from, 0) == next);
                previous = next;
            }
        }
    }
    check(Enzui::sceneTravelSeconds(0, 1) == 0.180);
    check(Enzui::sceneTravelSeconds(1, 0) == 0.140);
    check(Enzui::sceneTravelSeconds(0.1, 0) == 0.060);
    check(Enzui::sceneTravelSeconds(0.9, 1) == 0.060);
    check(Enzui::sceneProgress(0, 1, 0.09) > 0.85);
    for (auto from : {0.0, 0.25, 0.75, 1.0}) {
        for (auto goal : {0.0, 1.0}) {
            const auto duration = Enzui::sceneTravelSeconds(from, goal);
            check(Enzui::sceneProgress(from, goal, -1) == from);
            check(Enzui::sceneProgress(from, goal, 0) == from);
            check(Enzui::sceneProgress(from, goal, duration) == goal);
            check(Enzui::sceneProgress(from, goal, 1) == goal);
            auto previous = from;
            for (int step = 1; step <= 10; ++step) {
                const auto next = Enzui::sceneProgress(from, goal, duration * step / 10);
                check(next >= std::min(from, goal) && next <= std::max(from, goal));
                check(goal >= from ? next >= previous : next <= previous);
                previous = next;
            }
        }
    }
    for (auto rate : {25.0, 50.0, 100.0, 150.0, 200.0}) {
        for (auto from : {0.0, 0.25, 0.75, 1.0}) {
            for (auto goal : {0.0, 1.0}) {
                const auto baseline = Enzui::sceneTravelSeconds(from, goal);
                const auto scaled = Enzui::sceneTravelSeconds(from, goal, rate);
                check(std::abs(scaled - baseline * 100 / rate) < 0.000001);
                check(Enzui::sceneProgress(from, goal, scaled, rate) == goal);
                check(std::abs(Enzui::sceneProgress(from, goal, scaled / 2, rate) - Enzui::sceneProgress(from, goal, baseline / 2)) < 0.000001);
            }
        }
    }
    for (auto rate : {25.0, 50.0, 100.0, 150.0, 200.0}) {
        check(std::abs(Enzui::restoreTravelSeconds(0, rate) - .240 * 100 / rate) < .000001);
        for (auto from : {0.0, 0.25, 0.75, 1.0}) {
            const auto duration = Enzui::restoreTravelSeconds(from, rate);
            check(Enzui::restoreProgress(from, 0, rate) == from);
            check(Enzui::restoreProgress(from, duration, rate) == 1);
            auto previous = from;
            for (int step = 1; step <= 10; ++step) {
                const auto next = Enzui::restoreProgress(from, duration * step / 10, rate);
                check(next >= previous && next <= 1);
                check(Enzui::restoreProgress(next, 0, 200) == next);
                previous = next;
            }
        }
    }
    for (auto rate : {25.0, 50.0, 100.0, 200.0}) {
        for (auto from : {0.0, 1.0 / 3.0, 0.36, 1.0}) {
            for (auto goal : {0.0, 1.0 / 3.0, 0.36}) {
                const auto duration = Enzui::pocketTravelSeconds(from, goal, rate);
                check(duration >= .060 * 100 / rate && duration <= .180 * 100 / rate);
                check(Enzui::pocketProgress(from, goal, 0, rate) == from);
                check(Enzui::pocketProgress(from, goal, duration, rate) == goal);
                auto previous = from;
                for (int step = 1; step <= 10; ++step) {
                    auto next = Enzui::pocketProgress(from, goal, duration * step / 10, rate);
                    check(next >= std::min(from, goal) - .000001 && next <= std::max(from, goal) + .000001);
                    check(goal >= from ? next >= previous : next <= previous);
                    previous = next;
                }
            }
        }
    }
    for (auto rate : {0.0, -1.0, 201.0, std::numeric_limits<double>::infinity(), std::numeric_limits<double>::quiet_NaN()}) {
        check(Enzui::sceneTravelSeconds(0, 1, rate) == 0.180);
        check(Enzui::restoreTravelSeconds(0, rate) == 0.240);
    }
    for (auto width : {16.0, 36.0, 96.0}) {
        for (auto frame : {0.0, 3.0, 8.0}) {
            check(Enzui::railOffset(width, frame, 0) == width - frame);
            check(Enzui::railOffset(width, frame, 1) == 0);
            auto previous = width - frame;
            for (int step = 0; step <= 100; ++step) {
                const auto progress = step / 100.0;
                const auto offset = Enzui::railOffset(width, frame, progress);
                const auto visible = frame + std::round((width - frame) * progress);
                check(offset <= previous && offset >= 0);
                check(width - offset == visible);
                check(1440 - width + offset == 1440 - visible);
                previous = offset;
            }
        }
    }
    std::cout << checks << " scene-timing assertions passed\n";
}
