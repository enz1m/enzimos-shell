#include "../compositor/GlScissorState.hpp"
#include <array>
#include <cassert>
#include <iostream>

namespace {
struct FakeGl {
    static inline std::array<GLint, 4> box = {};
    static inline bool enabled = false;
    static GLboolean isEnabled(GLenum cap) { return cap == GL_SCISSOR_TEST && enabled; }
    static void getIntegerv(GLenum name, GLint* out) {
        assert(name == GL_SCISSOR_BOX);
        for (int i = 0; i < 4; ++i)
            out[i] = box[i];
    }
    static void scissor(GLint x, GLint y, GLsizei w, GLsizei h) { box = {x, y, w, h}; }
    static void enable(GLenum cap) { enabled |= cap == GL_SCISSOR_TEST; }
    static void disable(GLenum cap) { enabled &= cap != GL_SCISSOR_TEST; }
};

// The renderer's view of the scissor box: it calls glScissor only when the wanted box changes.
struct CachingRenderer {
    std::array<GLint, 4> cached = {};
    void scissor(std::array<GLint, 4> wanted) {
        if (wanted != cached) {
            FakeGl::scissor(wanted[0], wanted[1], wanted[2], wanted[3]);
            cached = wanted;
        }
        FakeGl::enable(GL_SCISSOR_TEST);
    }
};

void glassDraw() {
    const EnzMaterial::ScissorState<FakeGl> saved;
    FakeGl::disable(GL_SCISSOR_TEST);
    FakeGl::enable(GL_SCISSOR_TEST);
    FakeGl::scissor(1548, 40, 700, 6);
    FakeGl::disable(GL_SCISSOR_TEST);
}
}

int main() {
    CachingRenderer renderer;
    const std::array<GLint, 4> toast = {1548, 33, 708, 215};
    renderer.scissor(toast);
    glassDraw();
    renderer.scissor(toast);
    assert(FakeGl::box == toast);
    assert(FakeGl::enabled);

    FakeGl::disable(GL_SCISSOR_TEST);
    glassDraw();
    assert(FakeGl::box == toast);
    assert(!FakeGl::enabled);
    std::cout << "Glass passes hand back the renderer's scissor box and test state.\n";
}
