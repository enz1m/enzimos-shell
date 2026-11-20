#pragma once

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <vector>

namespace EnzMaterial::ReadbackPolicy {

using Clock = std::chrono::steady_clock;
constexpr size_t MAX_IN_FLIGHT = 16;
constexpr size_t MAX_SPARE_BUFFERS = 8;
constexpr auto MAX_WAIT = std::chrono::milliseconds(250);
constexpr int POLL_MS = 16;

struct Stamp {
    uint64_t lifetime = 0;
    uint64_t revision = 0;
    bool operator==(const Stamp&) const = default;
};

enum class Fence { Pending, Ready, Failed };
enum class Decision { Wait, Deliver, Stale, Failed };

inline Decision decide(bool current, Clock::duration age, Fence fence) {
    if (!current)
        return Decision::Stale;
    if (fence == Fence::Failed || age >= MAX_WAIT)
        return Decision::Failed;
    return fence == Fence::Ready ? Decision::Deliver : Decision::Wait;
}

inline bool canQueue(size_t global, size_t sameLifetime, bool disabled) {
    return !disabled && global < MAX_IN_FLIGHT && sameLifetime == 0;
}

template <typename Entry, typename Classify, typename Finish>
void collect(std::vector<Entry>& entries, Classify classify, Finish finish) {
    for (auto it = entries.begin(); it != entries.end();) {
        const auto decision = classify(*it);
        if (decision == Decision::Wait) {
            ++it;
        } else {
            finish(*it, decision);
            it = entries.erase(it);
        }
    }
}

}
