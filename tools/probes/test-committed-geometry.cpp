#include "../../compositor/CommittedGeometry.hpp"
#include <cassert>
#include <iostream>
#include <limits>
#include <utility>

int main() {
    for (double committed : {24., 28., 32., 36., 44., 56.}) {
        assert(Enzui::anchoredY(10, 32, committed, false) == 10);
        assert(Enzui::anchoredY(858, 32, committed, true) + committed == 890);
    }
    for (auto [buffer, window] : {std::pair{540., 540.}, std::pair{540., 436.}, std::pair{436., 436.}}) {
        const double painted = Enzui::paintedHeight(buffer, window);
        assert(Enzui::anchoredY(414, 436, painted, true) + painted == 850);
        assert(Enzui::anchoredY(414, 436, painted, false) == 414);
    }
    assert(Enzui::paintedHeight(436, 540) == 436);
    assert(Enzui::paintedHeight(540, 436) == 436);

    assert(Enzui::panelAnchorFits(998, 511, 458, 384, 1440, 900, false));
    assert(Enzui::panelAnchorFits(-28, -10, 458, 384, 1440, 900, false));
    assert(!Enzui::panelAnchorFits(-37, 0, 458, 384, 1440, 900, false));
    assert(!Enzui::panelAnchorFits(1030, 0, 458, 384, 1440, 900, false));
    assert(!Enzui::panelAnchorFits(0, 560, 458, 384, 1440, 900, false));
    assert(Enzui::panelAnchorFits(0, 858, 1440, 32, 1440, 900, true));
    assert(!Enzui::panelAnchorFits(-1, 858, 1440, 32, 1440, 900, true));
    assert(!Enzui::panelAnchorFits(0, 869, 1440, 32, 1440, 900, true));
    assert(!Enzui::panelAnchorFits(0, 858, 1439, 32, 1440, 900, true));
    assert(!Enzui::panelAnchorFits(0, 0, 1440, std::numeric_limits<double>::quiet_NaN(), 1440, 900, false));

    using Enzui::ContentGeometry;
    Enzui::CommittedGeometry cache;
    const ContentGeometry large{0, 0, 700, 600, 700, 600, 700, 600};
    const ContentGeometry resized{0, 0, 360, 520, 360, 520, 700, 600};
    assert(!cache.matches(large, 700, 600));
    cache.observe(large, false);
    assert(!cache.matches(large, 700, 600));
    cache.observe(large, true);
    assert(cache.matches(large, 700, 600));
    assert(!cache.matches(large, 360, 520));
    cache.observe(resized, false);
    assert(!cache.matches(resized, 360, 520));
    assert(cache.matches(large, 700, 600));
    cache.observe(resized, true);
    assert(cache.matches(resized, 360, 520));
    assert(!cache.matches(large, 700, 600));
    const ContentGeometry offset{5, 10, 360, 520, 360, 520, 370, 540};
    cache.observe(offset, true);
    assert(cache.matches(offset, 360, 520));
    auto bad = offset;
    bad.extentHeight = 529;
    cache.observe(bad, true);
    assert(!cache.matches(bad, 360, 520));
    bad = offset;
    bad.x = -1;
    cache.observe(bad, true);
    assert(!cache.matches(bad, 360, 520));
    bad = offset;
    bad.width = std::numeric_limits<double>::quiet_NaN();
    cache.observe(bad, true);
    assert(!cache.matches(bad, 360, 520));
    assert(Enzui::placementState(false, false, false) == Enzui::PlacementState::Pending);
    assert(Enzui::placementState(false, false, true) == Enzui::PlacementState::Failed);
    assert(Enzui::placementState(false, true, false) == Enzui::PlacementState::Failed);
    assert(Enzui::placementState(false, true, true) == Enzui::PlacementState::Failed);
    for (const bool presented : {false, true})
        for (const bool expired : {false, true})
            assert(Enzui::placementState(true, presented, expired) == Enzui::PlacementState::Ready);
    std::cout << "Committed geometry and presentation placement checks passed\n";
}
