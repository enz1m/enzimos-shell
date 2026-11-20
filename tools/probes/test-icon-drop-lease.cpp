#include "../../compositor/IconDropLease.hpp"
#include <cassert>
#include <limits>

int main() {
    Enzui::IconDropLease lease;
    lease.release(1, 300, 200);
    assert(!lease.take(true, 1, 300, 200));
    lease.press(false, 10, 200);
    lease.release(2, 300, 200);
    assert(!lease.take(true, 2, 300, 200));
    lease.press(true, 10, 200);
    assert(!lease.take(true, 3, 300, 200));
    lease.release(3, 18, 200);
    assert(!lease.take(true, 3, 18, 200));
    lease.press(true, 10, 200);
    lease.release(4, 300, 200);
    assert(lease.take(true, 4.2, 301, 200));
    assert(!lease.take(true, 4.2, 301, 200));
    lease.press(true, 10, 200);
    lease.cancel();
    lease.release(5, 300, 200);
    assert(!lease.take(true, 5, 300, 200));
    for (int scenario = 0; scenario < 5; ++scenario) {
        lease.press(true, 10, 200);
        lease.release(6, 300, 200);
        const double now = scenario == 0 ? 5 : scenario == 1 ? 7.01 : 6;
        const double x = scenario == 2 ? 304 : scenario == 3 ? std::numeric_limits<double>::quiet_NaN() : 300;
        assert(!lease.take(scenario != 4, now, x, 200));
        assert(!lease.pending);
    }
    lease.press(true, std::numeric_limits<double>::infinity(), 200);
    lease.release(8, 300, 200);
    assert(!lease.take(true, 8, 300, 200));
    lease.press(true, 10, 200);
    lease.release(9, 300, std::numeric_limits<double>::quiet_NaN());
    assert(!lease.take(true, 9, 300, 200));
}
