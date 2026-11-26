#pragma once
#include <GLES3/gl32.h>

namespace EnzMaterial {
// Restores the scissor box and test on scope exit. The renderer skips glScissor when the box it
// wants equals the box it last set, so a box left changed clips its following draws.
template <typename Gl>
class ScissorState {
  public:
    ScissorState() : enabled(Gl::isEnabled(GL_SCISSOR_TEST)) { Gl::getIntegerv(GL_SCISSOR_BOX, box); }
    ~ScissorState() {
        Gl::scissor(box[0], box[1], box[2], box[3]);
        enabled ? Gl::enable(GL_SCISSOR_TEST) : Gl::disable(GL_SCISSOR_TEST);
    }
    ScissorState(const ScissorState&) = delete;
    ScissorState& operator=(const ScissorState&) = delete;

  private:
    GLint box[4] = {};
    bool enabled;
};

struct LiveGl {
    static GLboolean isEnabled(GLenum cap) { return glIsEnabled(cap); }
    static void getIntegerv(GLenum name, GLint* out) { glGetIntegerv(name, out); }
    static void scissor(GLint x, GLint y, GLsizei w, GLsizei h) { glScissor(x, y, w, h); }
    static void enable(GLenum cap) { glEnable(cap); }
    static void disable(GLenum cap) { glDisable(cap); }
};
}
