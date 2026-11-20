class DetachTabPass final : public IPassElement {
  public:
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    bool disableSimplification() override { return true; }
    // Its rectangles clip to the frame's damage; a hover change damages the
    // tab and the grip (see the pointer listener).
    bool requiresFullDamage() override { return false; }
    const char* passName() override { return "EnzSidebarDetachTab"; }
    ePassElementType type() override { return EK_CUSTOM; }
    std::vector<UP<IPassElement>> draw() override {
        std::vector<UP<IPassElement>> elements;
        if (g_pHyprRenderer->m_renderData.pMonitor != output || !output)
            return elements;
        const auto box = detachTab();
        if (box.w <= 0)
            return elements;
        const bool hovered = box.containsPoint(g_pInputManager->getMouseCoordsInternal());
        const auto local = box.copy().translate(-output->m_position);
        auto rect = [&](CBox bounds, CHyprColor color, int radius) {
            elements.push_back(makeUnique<CRectPassElement>(CRectPassElement::SRectData{.box=bounds.copy().scale(output->m_scale).round(), .color=color, .round=int(std::round(radius * output->m_scale))}));
        };
        // At rest the handle is a slim grabber, a light bar on a soft dark
        // edge so it reads over light and dark panes alike; the full tab
        // appears, in the accent, only under the pointer. A grey slab with
        // three dashes sat heavily on the pane's own content.
        if (hovered)
            rect(local, CHyprColor{0.20F, 0.52F, 0.96F, 0.94F}, 8);
        else
            rect({local.x + 5, local.y + 17, 6, 38}, CHyprColor{0.F, 0.F, 0.F, 0.28F}, 3);
        rect({local.x + 6, local.y + 18, 4, 36}, CHyprColor{0.93F, 0.94F, 0.96F, hovered ? 0.98F : 0.82F}, 2);
        const auto grip = resizeGrip();
        if (grip.w > 0) {
            const auto localGrip = grip.copy().translate(-output->m_position);
            const auto accent = resizeHover ? CHyprColor{0.30F, 0.68F, 1.F, 1.F} : CHyprColor{0.62F, 0.64F, 0.69F, 0.70F};
            // Two arms of one length meeting in the corner the grip sits
            // in: the pane's outer bottom corner.
            rect({localGrip.x + (right ? 4 : 6), localGrip.y + 16, 12, 3}, accent, 1);
            rect({localGrip.x + (right ? 4 : 15), localGrip.y + 7, 3, 12}, accent, 1);
        }
        return elements;
    }
};
