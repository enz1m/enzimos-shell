#pragma once

#include <chrono>
#include <cstdint>
#include <limits>
#include <optional>

namespace Enzui {

// Each ticket belongs to one retained surface/request. The native adapter must
// observe applied commits, not configure acknowledgments or pending surface state.
class ContentReadiness {
  public:
    using Clock = std::chrono::steady_clock;
    using Time = Clock::time_point;
    using Ticket = uint64_t;
    enum class State { Idle, Waiting, Ready, TimedOut, Changed };
    struct Commit {
        bool ackedSize;
        bool windowGeometry;
        bool contentExtent;
        bool texture;
        bool attachedBuffer;
        bool matches() const { return ackedSize && windowGeometry && contentExtent && texture; }
    };

    std::optional<Ticket> begin(Time now) {
        if (ticket == std::numeric_limits<Ticket>::max()) {
            phase = State::Changed;
            return std::nullopt;
        }
        ++ticket;
        phase = State::Waiting;
        deadline = now + std::chrono::seconds(3);
        return ticket;
    }
    void cancel() { phase = State::Idle; }
    void advance(Time now) {
        if (phase == State::Waiting && now >= deadline)
            phase = State::TimedOut;
    }
    void observe(Ticket current, Commit commit, Time now) {
        if (current != ticket)
            return;
        advance(now);
        if (phase == State::Ready && !commit.matches())
            phase = State::Changed;
        else if (phase == State::Waiting && commit.matches() && commit.attachedBuffer)
            phase = State::Ready;
    }
    State state() const { return phase; }
    bool ready() const { return phase == State::Ready; }

  private:
    Ticket ticket = 0;
    State phase = State::Idle;
    Time deadline{};
};
}
