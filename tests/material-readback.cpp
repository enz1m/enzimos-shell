#include "../compositor/MaterialReadback.hpp"
#include <cassert>
#include <iostream>

using namespace EnzMaterial::ReadbackPolicy;
using namespace std::chrono_literals;

int main() {
    assert(decide(true, 0ms, Fence::Failed) == Decision::Failed);
    assert(decide(true, 249ms, Fence::Pending) == Decision::Wait);
    assert(decide(true, 250ms, Fence::Pending) == Decision::Failed);
    assert(decide(true, 250ms, Fence::Ready) == Decision::Failed);
    assert(decide(true, 0ms, Fence::Ready) == Decision::Deliver);
    assert(decide(false, 0ms, Fence::Ready) == Decision::Stale);
    assert(decide(false, 0ms, Fence::Failed) == Decision::Stale);
    assert(canQueue(MAX_IN_FLIGHT - 1, 0, false));
    assert(!canQueue(MAX_IN_FLIGHT, 0, false));
    assert(!canQueue(0, 1, false));
    assert(!canQueue(0, 0, true));

    struct Entry { int id; bool current; Clock::duration age; Fence fence; };
    std::vector<Entry> queue = {{1, true, 0ms, Fence::Pending}, {2, true, 0ms, Fence::Failed}, {3, true, 0ms, Fence::Ready}, {4, false, 0ms, Fence::Ready}};
    std::vector<int> completed;
    bool disabled = false;
    collect(queue, [](const auto& entry) { return decide(entry.current, entry.age, entry.fence); }, [&](const auto& entry, auto decision) {
        completed.push_back(entry.id);
        disabled |= decision == Decision::Failed;
    });
    assert((completed == std::vector<int>{2, 3, 4}));
    assert(queue.size() == 1 && queue[0].id == 1);
    assert(disabled);
    queue[0].age = MAX_WAIT;
    collect(queue, [](const auto& entry) { return decide(entry.current, entry.age, entry.fence); }, [](const auto&, auto decision) { assert(decision == Decision::Failed); });
    assert(queue.empty());

    const Stamp sample{1, 8};
    for (const auto current : {Stamp{1, 9}, Stamp{2, 8}, Stamp{0, 0}}) {
        assert(sample != current);
        assert(decide(sample == current, 0ms, Fence::Ready) == Decision::Stale);
    }
    assert(decide(sample == Stamp{1, 8}, 0ms, Fence::Ready) == Decision::Deliver);
    std::cout << "Material readbacks: failure, deadline, bounded admission, no head-of-line blocking, revision and lifetime refusal pass.\n";
}
