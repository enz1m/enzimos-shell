#include "../../compositor/FloatingChrome.hpp"
#include <cassert>
#include <limits>

int main() {
    for (const bool floating : {false, true}) {
        for (const bool restoring : {false, true})
            assert(Enzui::protectedContentPadding(floating, restoring) == (floating && !restoring ? 0 : 6));
        for (int inset = -1; inset <= 16; ++inset)
            assert(Enzui::validPaneInset(floating, inset) == (inset == 6 || (floating && inset >= 7 && inset <= 10)));
        assert(!Enzui::validPaneInset(floating, 6.5));
        assert(!Enzui::validPaneInset(floating, std::numeric_limits<double>::quiet_NaN()));
        assert(!Enzui::validPaneInset(floating, std::numeric_limits<double>::infinity()));
    }
    for (const int width : {32, 40, 64, 128, 256}) {
        for (const int height : {20, 48, 80, 618, 900}) {
            const auto spans = Enzui::capsuleInput(width, height);
            assert(!spans.empty() && spans.size() <= 257);
            for (int y = 0; y < height; ++y) {
                for (int x = 0; x < width; ++x) {
                    int covered = 0;
                    for (const auto& span : spans)
                        covered += x >= span.x && x < span.x + span.width && y >= span.y && y < span.y + span.height;
                    assert(covered <= 1);
                    assert(bool(covered) == Enzui::capsuleContains(width, height, x + .5, y + .5));
                }
            }
            assert(Enzui::capsuleContains(width, height, width / 2., height / 2.));
            assert(!Enzui::capsuleContains(width, height, 0, 0));
            assert(!Enzui::capsuleContains(width, height, width, height / 2.));
        }
    }
    assert(Enzui::capsuleInput(0, 10).empty());
    assert(Enzui::capsuleInput(40, 16385).empty());
    assert(!Enzui::capsuleContains(40, 48, std::numeric_limits<double>::quiet_NaN(), 1));
}
