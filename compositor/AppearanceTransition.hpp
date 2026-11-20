// Included in EnzMaterial after its GL helpers. The previous complete window
// plane is retained while clients commit a new appearance, then dissolved over
// the live scene. This keeps independently committed windows out of mixed-theme
// frames.
namespace AppearanceTransition {
using Clock = std::chrono::steady_clock;
struct Output {
  Target displayed, previous;
  WP<Monitor::CMonitor> monitor;
  bool valid = false, begin = false, active = false;
  Clock::time_point started{}, ready{};
};
static std::map<MONITORID, Output> outputs;
static CHyprSignalListener listener;
static void damage() {
  for (const auto &mon : State::monitorState()->monitors())
    renderer().damageMonitor(mon);
}
static SDispatchResult command(const std::string &argument) {
  const auto now = Clock::now();
  if (argument != "begin" && argument != "ready" && argument != "cancel")
    return {.success = false, .error = "expected begin, ready or cancel"};
  for (auto &[id, output] : outputs) {
    if (argument == "begin") {
      output.begin = true;
      output.started = now;
      output.ready = {};
    } else if (argument == "ready") {
      if (output.begin || output.active)
        output.ready = now;
    } else {
      output.begin = output.active = false;
    }
  }
  damage();
  return {};
}
class Pass : public IPassElement {
public:
  bool needsLiveBlur() override { return false; }
  bool needsPrecomputeBlur() override { return false; }
  bool disableSimplification() override { return true; }
  bool undiscardable() override { return true; }
  bool requiresFullDamage() override {
    const auto mon = renderer().m_renderData.pMonitor.lock();
    const auto it = mon ? outputs.find(mon->m_id) : outputs.end();
    return it == outputs.end() || !it->second.valid || it->second.begin ||
           it->second.active;
  }
  const char *passName() override { return "EnzAppearanceTransition"; }
  ePassElementType type() override { return EK_CUSTOM; }
  std::vector<UP<IPassElement>> draw() override {
    const auto mon = renderer().m_renderData.pMonitor.lock();
    const auto current = renderer().m_renderData.currentFB;
    const auto fb =
        current ? dynamic_cast<Render::GL::CGLFramebuffer *>(current.get())
                : nullptr;
    if (!mon || !fb || fb->m_size.x < 1 || fb->m_size.y < 1 ||
        fb->m_size.x * fb->m_size.y > 8388608 || !gl.ready())
      return {};
    for (auto it = outputs.begin(); it != outputs.end();) {
      if (it->second.monitor.expired()) {
        it->second.displayed.release();
        it->second.previous.release();
        it = outputs.erase(it);
      } else
        ++it;
    }
    auto &o = outputs[mon->m_id];
    GLint read, draw, program, vao, activeTexture, texture, viewport[4], src,
        dst, srcA, dstA;
    glGetIntegerv(GL_READ_FRAMEBUFFER_BINDING, &read);
    glGetIntegerv(GL_DRAW_FRAMEBUFFER_BINDING, &draw);
    glGetIntegerv(GL_CURRENT_PROGRAM, &program);
    glGetIntegerv(GL_VERTEX_ARRAY_BINDING, &vao);
    glGetIntegerv(GL_ACTIVE_TEXTURE, &activeTexture);
    glActiveTexture(GL_TEXTURE0);
    glGetIntegerv(GL_TEXTURE_BINDING_2D, &texture);
    glGetIntegerv(GL_VIEWPORT, viewport);
    glGetIntegerv(GL_BLEND_SRC_RGB, &src);
    glGetIntegerv(GL_BLEND_DST_RGB, &dst);
    glGetIntegerv(GL_BLEND_SRC_ALPHA, &srcA);
    glGetIntegerv(GL_BLEND_DST_ALPHA, &dstA);
    const bool blend = glIsEnabled(GL_BLEND),
               scissor = glIsEnabled(GL_SCISSOR_TEST),
               stencil = glIsEnabled(GL_STENCIL_TEST);
    glDisable(GL_SCISSOR_TEST);
    glDisable(GL_STENCIL_TEST);
    const int w = fb->m_size.x, h = fb->m_size.y;
    if (o.monitor.lock() != mon || o.displayed.w != w || o.displayed.h != h) {
      o.valid = false;
      o.active = false;
    }
    o.monitor = mon;
    if (o.begin) {
      o.begin = false;
      o.active = o.valid;
      if (o.active)
        std::swap(o.displayed, o.previous);
    }
    o.displayed.ensureExact(w, h);
    glBindFramebuffer(GL_DRAW_FRAMEBUFFER, fb->getFBID());
    if (o.active) {
      const auto now = Clock::now();
      // A failed save or disconnected client cannot leave an output frozen.
      if (o.ready == Clock::time_point{} &&
          now - o.started > std::chrono::seconds(2))
        o.ready = now;
      float t = o.ready == Clock::time_point{}
                    ? 0.F
                    : std::chrono::duration<float>(now - o.ready).count();
      t = std::clamp((t - 0.12F) / 0.32F, 0.F, 1.F);
      const float opacity = 1.F - t * t * (3.F - 2.F * t);
      if (opacity > 0.F) {
        glViewport(0, 0, w, h);
        glEnable(GL_BLEND);
        glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
        glUseProgram(gl.fade);
        glBindVertexArray(gl.vao);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, o.previous.texture);
        glUniform1i(glGetUniformLocation(gl.fade, "fade_texture"), 0);
        glUniform2f(glGetUniformLocation(gl.fade, "origin"), 0, 0);
        glUniform1f(glGetUniformLocation(gl.fade, "opacity"), opacity);
        glDrawArrays(GL_TRIANGLES, 0, 6);
      } else
        o.active = false;
      renderer().damageMonitor(mon);
    }
    // Cache the displayed composite, so a rapid reversal starts from exactly
    // what the user saw, including an unfinished dissolve.
    glBindFramebuffer(GL_READ_FRAMEBUFFER, fb->getFBID());
    glBindFramebuffer(GL_DRAW_FRAMEBUFFER, o.displayed.framebuffer);
    // Only copy repainted pixels. Retained pixels in the work buffer may
    // still contain last frame's cursor or overlay, drawn after this pass.
    if (!o.valid || o.active) {
      glBlitFramebuffer(0, 0, w, h, 0, 0, w, h, GL_COLOR_BUFFER_BIT,
                        GL_NEAREST);
    } else {
      for (const auto &r : renderer().m_renderData.damage.getRects())
        glBlitFramebuffer(r.x1, r.y1, r.x2, r.y2, r.x1, r.y1, r.x2, r.y2,
                          GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    o.valid = true;
    glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
    glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw);
    glUseProgram(program);
    glBindVertexArray(vao);
    glActiveTexture(GL_TEXTURE0);
    glBindTexture(GL_TEXTURE_2D, texture);
    glActiveTexture(activeTexture);
    glViewport(viewport[0], viewport[1], viewport[2], viewport[3]);
    glBlendFuncSeparate(src, dst, srcA, dstA);
    blend ? glEnable(GL_BLEND) : glDisable(GL_BLEND);
    scissor ? glEnable(GL_SCISSOR_TEST) : glDisable(GL_SCISSOR_TEST);
    stencil ? glEnable(GL_STENCIL_TEST) : glDisable(GL_STENCIL_TEST);
    return {};
  }
};
static int request(lua_State *state) {
  const auto result = command(luaL_checkstring(state, 1));
  lua_pushboolean(state, result.success);
  return 1;
}
static void init(HANDLE handle) {
  if (!HyprlandAPI::addLuaFunction(handle, "enzui_appearance", "transition",
                                   request))
    throw std::runtime_error(
        "enzui-material: cannot register appearance presentation");
  listener = Event::bus()->m_events.render.stage.listen([](eRenderStage stage) {
    if (stage == RENDER_POST_WINDOWS)
      renderer().addPassElement(makeUnique<Pass>());
  });
}
static void exit(HANDLE handle) {
  listener.reset();
  HyprlandAPI::removeLuaFunction(handle, "enzui_appearance", "transition");
  for (auto &[id, o] : outputs) {
    o.displayed.release();
    o.previous.release();
  }
  outputs.clear();
}
} // namespace AppearanceTransition
