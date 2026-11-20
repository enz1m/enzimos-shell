#pragma once
#include "RemoteSeat.hpp"
#include "src/desktop/view/window/Window.hpp"
#include "src/workspace/HLWorkspace.hpp"
#include "../vendor/json/single_include/nlohmann/json_fwd.hpp"
#include <chrono>
#include <string>
#include <vector>

namespace Enzremote {

struct SViewport { uint32_t width = 0, height = 0, refresh = 60; double scale = 1; };

class CDesktopLease {
  public:
    enum class Phase { PREPARING, CONFIGURING, ACTIVE, RESTORING, RESTORED, CLOSED, FAILED };
    CDesktopLease(PHLWINDOW window, SViewport viewport, CRemoteSeat& seat);
    ~CDesktopLease();
    void prepare();
    nlohmann::json description(bool secrets = false) const;
    bool acceptsUI(const std::string&) const;
    bool acceptsInput(const std::string&) const;
    void bindUI(int pid);
    void activate();
    void resize(SViewport);
    void contentSize(uint32_t width, uint32_t height, double scale);
    void restore(const std::string& reason);
    void tick();
    void windowClosed();
    bool inputAllowed() const;
    bool finished() const;
    bool uiReady() const;
    bool ownsWindow(PHLWINDOW) const;
    void windowMapped(PHLWINDOW);
    std::vector<PHLWINDOW> inputWindows() const;
    PHLWINDOW inputWindow() const;
    void focusInput(PHLWINDOW);
    PHLWINDOW window() const;
    PHLMONITOR remoteMonitor() const;
    Phase phase() const;

  private:
    PHLWINDOWREF m_window;
    PHLWINDOWREF m_placeholder;
    PHLWINDOWREF m_restoreFocus;
    PHLWORKSPACE m_originalWorkspace;
    PHLMONITORREF m_originalMonitor;
    PHLMONITORREF m_remoteMonitor;
    CRemoteSeat& m_seat;
    SViewport m_viewport;
    CBox m_originalBox;
    CBox m_restoreBox;
    bool m_originalFloating = true;
    bool m_restoreFloating = true;
    int m_originalInternalFS = 0, m_originalClientFS = 0;
    int m_restoreInternalFS = 0, m_restoreClientFS = 0;
    int m_uiPid = -1;
    Phase m_phase = Phase::PREPARING;
    std::chrono::steady_clock::time_point m_deadline;
    std::string m_id, m_uiToken, m_inputToken, m_snapshotPath, m_directory, m_reason;
    std::string m_mobileOutput, m_mobileWorkspace, m_stagingWorkspace;
    bool m_moved = false;
    bool m_createdOutput = false;
    unsigned m_stableFrames = 0, m_matchingCommits = 0;
    uint64_t m_bufferCommits = 0, m_observedCommit = 0;
    CHyprSignalListener m_committed;
    struct SDialog {
        PHLWINDOWREF window;
        Vector2D desktopSize;
        Vector2D desktopOffset;
        bool existing = false, allowedOverFullscreen = true;
        CBox target;
        unsigned stable = 0;
        bool ready = false;
        std::chrono::steady_clock::time_point deadline;
    };
    std::vector<SDialog> m_dialogs;
    PHLWINDOWREF m_inputWindow;
    uint32_t m_contentHeight = 0;

    void placeDialogs(bool restoring);
    bool settleDialogs(bool restoring);
    void returnDialogs();
    void resetFrameBarrier();
    bool frameReady(bool matches);
    void configureOutput();
    void tag(PHLWINDOW, const std::string&);
    void geometry(PHLWINDOW, const CBox&);
    void move(PHLWINDOW, const std::string&);
    void fullscreen(PHLWINDOW, int internal, int client);
    void floating(PHLWINDOW, bool);
    bool matchesFrame(PHLWINDOW, Vector2D logical, double scale) const;
    void finishRestore(bool settled);
    void cleanupOutput();
};

}
