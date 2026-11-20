struct IconFeedback {
    bool active = false, light = false;
    pid_t owner = 0;
    PHLWINDOWREF source, application;
    PHLMONITORREF monitor;
    double progress = 0, from = 0, goal = 0;
    Vector2D point, origin;
    Time::steady_tp began;
};
static IconFeedback iconFeedback;
static CHyprSignalListener iconMotionListener;
static void sampleIconFeedback() {
    auto& f = iconFeedback;
    if (!f.active) return;
    if (f.owner != ownerPid || !ownerAlive() || f.monitor != output || !f.source || !f.source->mapped() || f.source != railSource() ||
        !rail.configured || !f.application || !f.application->mapped() || remote(f.application.lock()) || f.application->m_monitor != output) {
        f = {};
        return;
    }
    const auto now = Time::steadyNow();
    f.progress = Enzui::sceneProgress(f.from, f.goal, std::chrono::duration<double>(now - f.began).count(), motionRate);
    const bool held = iconDrop.held && iconDrop.armed;
    if (!held && f.goal != 0) { f.from = f.progress; f.goal = 0; f.began = now; }
    if (held) f.point = g_pInputManager->getMouseCoordsInternal() - output->m_position;
    if (f.progress != f.goal) damage();
    if (!held && f.progress == 0) f = {};
}
static int beginIconFeedback(lua_State* L) {
    if (lua_gettop(L) != 5 || !motionOwner(L) || !lua_isboolean(L, 2) || !lua_isstring(L, 3) || !lua_isnumber(L, 4) || !lua_isnumber(L, 5))
        return luaL_error(L, "Invalid icon feedback");
    const auto point = g_pInputManager->getMouseCoordsInternal();
    const auto application = windowAddress(lua_tostring(L, 3));
    const Vector2D origin{lua_tonumber(L, 4), lua_tonumber(L, 5)};
    if (!application || application->backend().pid() == ownerPid || application->m_monitor != output || remote(application) ||
        !std::isfinite(origin.x) || !std::isfinite(origin.y) || std::hypot(origin.x - iconDrop.pressX, origin.y - iconDrop.pressY) > 64 ||
        !iconDrop.held || !iconDrop.armed || iconDropOwner != ownerPid || iconDropOutput != output || iconDropSource != railSource() ||
        std::hypot(point.x - iconDrop.pressX, point.y - iconDrop.pressY) <= 8) return result(L, false);
    iconFeedback = {};
    auto& f = iconFeedback;
    f.active = true; f.light = lua_toboolean(L, 2); f.owner = ownerPid;
    f.monitor = output; f.source = iconDropSource; f.application = application;
    f.point = point - output->m_position; f.origin = origin - output->m_position;
    f.goal = 1; f.began = Time::steadyNow();
    damage();
    return result(L, true);
}
class IconFeedbackPass final : public IPassElement {
  public:
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    bool disableSimplification() override { return true; }
    bool requiresFullDamage() override { return true; }
    const char* passName() override { return "EnzSidebarIconFeedback"; }
    ePassElementType type() override { return EK_CUSTOM; }
    std::vector<UP<IPassElement>> draw() override {
        auto& f = iconFeedback;
        std::vector<UP<IPassElement>> elements;
        if (!f.active || !ownerAlive() || !f.source || !f.source->mapped() || f.source != railSource() || f.owner != ownerPid ||
            f.monitor != output || g_pHyprRenderer->m_renderData.pMonitor != output) return elements;
        const float a = f.progress;
        const CHyprColor ink = f.light ? CHyprColor{0.12F,0.14F,0.18F,a} : CHyprColor{0.94F,0.95F,0.98F,a};
        const CHyprColor bg = f.light ? CHyprColor{0.98F,0.98F,0.99F,a} : CHyprColor{0.15F,0.16F,0.19F,a};
        const CHyprColor accent{0.20F,0.52F,0.96F,a};
        const auto point = f.goal == 1 ? f.point : f.origin + (f.point - f.origin) * f.progress;
        const bool outside = point.x < rail.box.x - 24 || point.x >= rail.box.x + rail.box.w + 24 || point.y < rail.box.y || point.y >= rail.box.y + rail.box.h;
        auto rect = [&](CBox box, CHyprColor color, int radius) {
            elements.push_back(makeUnique<CRectPassElement>(CRectPassElement::SRectData{.box=box.copy().scale(output->m_scale).round(), .color=color, .round=int(std::round(radius * output->m_scale))}));
        };
        if (outside && f.goal == 1) {
            if (auto plan = tilePlan(f.application.lock(), point + output->m_position)) {
                const auto box = plan->box.copy().translate(-output->m_position).expand(-6);
                rect(box, CHyprColor{0.20F,0.52F,0.96F,a*0.22F}, 12);
                rect({box.x+12,box.y,box.w-24,2},accent,1); rect({box.x+12,box.y+box.h-2,box.w-24,2},accent,1);
                rect({box.x,box.y+12,2,box.h-24},accent,1); rect({box.x+box.w-2,box.y+12,2,box.h-24},accent,1);
            }
        }
        rect({f.origin.x-18,f.origin.y-18,36,36},CHyprColor{0.20F,0.52F,0.96F,a*(outside?0.32F:0.18F)},18);
        rect({f.origin.x+7,f.origin.y-23,16,16},accent,8);
        rect({f.origin.x+11,f.origin.y-16,8,2},CHyprColor{1.F,1.F,1.F,a},1);
        const double x = std::clamp(point.x + (rail.right ? -52 : 12), 4.0, output->m_size.x - 44);
        const double y = std::clamp(point.y + 12 + (1-f.progress)*8, 4.0, output->m_size.y - 44);
        rect({x,y+3,40,40},CHyprColor{0.F,0.F,0.F,a*0.18F},15);
        rect({x,y,40,40},outside?accent:ink,15);
        rect({x+1,y+1,38,38},bg,14);
        rect({x+9,y+9,22,22},ink,5);
        rect({x+11,y+15,18,14},bg,2);
        rect({x+23,y+25,14,14},accent,7);
        rect({x+26,y+31,8,2},CHyprColor{1.F,1.F,1.F,a},1);
        return elements;
    }
};
