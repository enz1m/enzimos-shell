namespace AppearanceTransition {
using Clock = std::chrono::steady_clock;
struct Output {
  Target previous;
  bool capture = false, active = false;
  Clock::time_point started{}, ready{};
  void release() {
    previous.release();
    capture = active = false;
  }
};
static std::map<MONITORID, Output> outputs;
static CHyprSignalListener listener;
static bool pendingCapture() {
  return std::ranges::any_of(outputs,
                             [](const auto &entry) { return entry.second.capture; });
}
// Runs inside a render, where the GL context that owns the targets is current.
static void releaseIdleOutputs() {
  std::erase_if(outputs, [](auto &entry) {
    auto &o = entry.second;
    const bool present = std::ranges::any_of(
        State::monitorState()->monitors(),
        [&](const auto &mon) { return mon && mon->m_id == entry.first; });
    if (!present || (!o.capture && !o.active)) {
      o.release();
      return true;
    }
    return false;
  });
}
// The caller waits for `captured` before writing the new theme.
static std::string command(const std::string &argument) {
  if (argument == "begin") {
    for (const auto &mon : State::monitorState()->monitors()) {
      if (!mon || !mon->m_enabled || !mon->m_dpmsStatus)
        continue;
      outputs[mon->m_id].capture = true;
      renderer().damageMonitor(mon);
    }
    return pendingCapture() ? "pending" : "ready";
  }
  if (argument == "captured")
    return pendingCapture() ? "pending" : "ready";
  if (argument == "ready") {
    for (auto &[id, o] : outputs) {
      // A capture taken after the theme write would show the new appearance.
      o.capture = false;
      if (o.active)
        o.ready = Clock::now();
    }
  } else if (argument == "cancel") {
    for (auto &[id, o] : outputs)
      o.capture = o.active = false;
  } else
    return {};
  for (const auto &mon : State::monitorState()->monitors())
    if (mon)
      renderer().damageMonitor(mon);
  return "ready";
}
class Pass : public IPassElement {
public:
  bool needsLiveBlur() override { return false; }
  bool needsPrecomputeBlur() override { return false; }
  bool undiscardable() override { return true; }
  // The capture copies the whole composite, so every pixel must be repainted.
  bool requiresFullDamage() override { return true; }
  const char *passName() override { return "EnzAppearanceTransition"; }
  ePassElementType type() override { return EK_CUSTOM; }
  std::vector<UP<IPassElement>> draw() override {
    const auto mon = renderer().m_renderData.pMonitor.lock();
    if (!mon)
      return {};
    const auto it = outputs.find(mon->m_id);
    if (it == outputs.end())
      return {};
    auto &o = it->second;
    const auto current = renderer().m_renderData.currentFB;
    const auto fb =
        current ? dynamic_cast<Render::GL::CGLFramebuffer *>(current.get())
                : nullptr;
    if (!fb || fb->m_size.x < 1 || fb->m_size.y < 1 ||
        fb->m_size.x * fb->m_size.y > 8388608 || !gl.ready()) {
      o.release();
      return {};
    }
    const int w = fb->m_size.x, h = fb->m_size.y;
    if (o.active && (o.previous.w != w || o.previous.h != h)) {
      o.active = false;
      o.previous.release();
    }
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
        renderer().damageMonitor(mon);
      } else {
        o.active = false;
        o.previous.release();
      }
    }
    // Capture the visible composite so a reversal can continue a partial dissolve.
    if (o.capture) {
      Target captured;
      captured.ensureExact(w, h);
      glBindFramebuffer(GL_READ_FRAMEBUFFER, fb->getFBID());
      glBindFramebuffer(GL_DRAW_FRAMEBUFFER, captured.framebuffer);
      glBlitFramebuffer(0, 0, w, h, 0, 0, w, h, GL_COLOR_BUFFER_BIT,
                        GL_NEAREST);
      std::swap(o.previous, captured);
      captured.release();
      o.capture = false;
      o.active = true;
      o.started = Clock::now();
      o.ready = {};
      renderer().damageMonitor(mon);
    }
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
  if (result.empty())
    lua_pushboolean(state, false);
  else
    lua_pushstring(state, result.c_str());
  return 1;
}
static void init(HANDLE handle) {
  if (!HyprlandAPI::addLuaFunction(handle, "enzui_appearance", "transition",
                                   request))
    throw std::runtime_error(
        "enzui-material: cannot register appearance presentation");
  listener = Event::bus()->m_events.render.stage.listen([](eRenderStage stage) {
    if (stage == RENDER_PRE && !outputs.empty())
      releaseIdleOutputs();
    if (stage != RENDER_POST_WINDOWS)
      return;
    const auto mon = renderer().m_renderData.pMonitor.lock();
    const auto it = mon ? outputs.find(mon->m_id) : outputs.end();
    if (it != outputs.end() && (it->second.capture || it->second.active))
      renderer().addPassElement(makeUnique<Pass>());
  });
}
static void exit(HANDLE handle) {
  listener.reset();
  HyprlandAPI::removeLuaFunction(handle, "enzui_appearance", "transition");
  for (auto &[id, o] : outputs)
    o.release();
  outputs.clear();
}
} // namespace AppearanceTransition
