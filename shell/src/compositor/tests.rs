use std::path::PathBuf;
use std::time::Duration;

use super::*;

use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn compositor_async_behaviour() {
    // Protocol rejection is not a successful action.
    {
        let compositor =
            Compositor::new(PathBuf::from("/usr/bin/true"), "nested-test".into()).unwrap();
        assert!(compositor.open_terminal().await.is_err());
        assert!(compositor.focus_notification_center().await.is_err());
        assert!(compositor.switch_workspace(2).await.is_err());
        assert!(Compositor::new(PathBuf::from("hyprctl"), "nested-test".into()).is_err());
        assert!(
            Compositor::new(PathBuf::from("/usr/bin/true"), "../other-instance".into()).is_err()
        );
    }
    // A stuck control process is bounded.
    {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("hyprctl");
        std::fs::write(&executable, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let compositor = Compositor::new(executable, "nested-test".into()).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            compositor.open_terminal().await,
            Err("Compositor did not respond".into())
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    // Requests are scoped and workspace results are read back.
    {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("hyprctl");
        let record = directory.path().join("requests");
        std::fs::write(&executable, format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$3\" in -j) printf '{{\"id\":3}}';; --batch) printf false;; eval) printf ok;; esac\n", record.display()
            )).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let compositor = Compositor::new(executable.clone(), "nested-test".into()).unwrap();
        assert_eq!(compositor.switch_workspace(3).await.unwrap().workspace, 3);
        compositor.open_terminal().await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&record).unwrap(),
            "-i nested-test eval return hl.dispatch(hl.dsp.focus({workspace=3}))\n-i nested-test -j activeworkspace\n-i nested-test --batch repl return tostring(enzui_desktop_tiling_at and enzui_desktop_tiling_at(3) or (enzui_desktop_tiling_at == nil and enzui_desktop.tiling == true))\n-i nested-test -j devices\n-i nested-test eval return hl.dispatch(hl.dsp.exec_cmd('exec \"${ENZUI_TERMINAL:-foot}\"'))\n"
        );
        let active = directory.path().join("active-window");
        std::fs::write(&executable, format!(
            "#!/bin/sh\ncase \"$4\" in activewindow) cat '{}';; clients) printf '[{{\"pid\":42,\"mapped\":true,\"xdgTag\":\"origin\"}}]';; esac\n", active.display()
        )).unwrap();
        std::fs::write(&active, "invalid window response").unwrap();
        assert!(
            compositor.activity_windows().await.is_err(),
            "malformed focus cannot mean no focused window"
        );
        std::fs::write(&active, "[]").unwrap();
        assert!(
            compositor.activity_windows().await.is_err(),
            "a non-object is not an active-window observation"
        );
        std::fs::write(&active, r#"{"pid":42,"xdgTag":"origin"}"#).unwrap();
        let windows = compositor.activity_windows().await.unwrap();
        assert_eq!(windows.active, Some((42, Some("origin".into()))));
        assert_eq!(windows.tagged, vec![(42, "origin".into())]);
        assert!(compositor.switch_workspace(0).await.is_err());
        // A close completion must not steal focus from a popup opened meanwhile.
        std::fs::write(&executable, "#!/bin/sh\ncase \"$4\" in activewindow) printf '%s' '{\"class\":\"dev.enzui.ControlCenter\",\"mapped\":true,\"visible\":true}';; *) exit 71;; esac\n").unwrap();
        compositor.return_focus_to_app().await.unwrap();
        assert!(Compositor::new(executable, "".into()).is_err());
    }
}
