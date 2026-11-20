#include "../../compositor/RestorePicture.hpp"
#include <cassert>
#include <iostream>
#include <limits>

int main() {
    unsigned checks = 0;
    auto check = [&](bool value) { assert(value); ++checks; };
    for (auto source : {std::pair{360.0, 640.0}, std::pair{620.0, 480.0}, std::pair{1024.0, 768.0}}) {
        for (auto target : {std::pair{360.0, 640.0}, std::pair{480.0, 560.0}, std::pair{620.0, 480.0}, std::pair{1024.0, 768.0}}) {
            const auto crop = Enzui::coverCrop(source.first, source.second, target.first, target.second);
            check(crop.has_value());
            check(crop->right > 0 && crop->right <= 1);
            check(crop->bottom > 0 && crop->bottom <= 1);
            check(crop->right == 1 || crop->bottom == 1);
            check(std::abs(source.first * crop->right / target.first - source.second * crop->bottom / target.second) < .000001);
        }
    }
    for (const auto invalid : {0.0, -1.0, std::numeric_limits<double>::infinity(), std::numeric_limits<double>::quiet_NaN()}) {
        check(!Enzui::coverCrop(invalid, 640, 620, 480));
        check(!Enzui::coverCrop(360, invalid, 620, 480));
        check(!Enzui::coverCrop(360, 640, invalid, 480));
        check(!Enzui::coverCrop(360, 640, 620, invalid));
    }
    std::cout << checks << " restore-picture assertions passed\n";
}
