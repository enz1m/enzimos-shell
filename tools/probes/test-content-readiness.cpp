#include "../../compositor/ContentReadiness.hpp"
#include <cassert>
#include <chrono>
#include <iostream>

using Gate = Enzui::ContentReadiness;
using namespace std::chrono_literals;

int main() {
    const Gate::Time now{};
    const Gate::Commit complete{true, true, true, true, true};
    unsigned checks = 0;
    for (unsigned bits = 0; bits < 32; ++bits) {
        Gate gate;
        const auto ticket = *gate.begin(now);
        gate.observe(
            ticket,
            {bool(bits & 1), bool(bits & 2), bool(bits & 4), bool(bits & 8), bool(bits & 16)},
            now + 1s);
        assert(gate.ready() == (bits == 31));
        ++checks;
    }
    Gate gate;
    const auto first = *gate.begin(now);
    const auto second = *gate.begin(now + 1s);
    gate.observe(first, complete, now + 2s);
    assert(gate.state() == Gate::State::Waiting);
    gate.observe(second, complete, now + 2s);
    assert(gate.ready());
    gate.observe(second, {true, true, true, true, false}, now + 20s);
    assert(gate.ready());
    gate.observe(second, {true, false, true, true, true}, now + 21s);
    assert(gate.state() == Gate::State::Changed);
    gate.observe(second, complete, now + 22s);
    assert(!gate.ready());
    const auto third = *gate.begin(now + 23s);
    gate.observe(third, complete, now + 26s);
    assert(gate.state() == Gate::State::TimedOut);
    gate.observe(third, complete, now + 27s);
    assert(!gate.ready());
    const auto fourth = *gate.begin(now + 28s);
    gate.observe(fourth, complete, now + 29s);
    assert(gate.ready());
    gate.cancel();
    gate.observe(fourth, complete, now + 30s);
    assert(gate.state() == Gate::State::Idle);
    gate.begin(now + 31s);
    gate.advance(now + 34s);
    assert(gate.state() == Gate::State::TimedOut);
    checks += 10;
    std::cout << checks << " content-readiness assertions passed\n";
}
