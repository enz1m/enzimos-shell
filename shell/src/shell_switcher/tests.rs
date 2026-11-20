use super::*;

fn shell() -> Shell {
    let compositor = enzui_shell::compositor::Compositor::new(
        std::path::PathBuf::from("/usr/bin/true"),
        "fixture".into(),
    )
    .unwrap();
    Shell::new(compositor, std::path::PathBuf::from("/")).0
}
#[test]
fn shell_switcher_behaviour() {
    // State reads coalesce and older snapshot cannot reopen after release.
    {
        let mut shell = shell();
        let _ = shell.switcher_command(Command::SwitchState);
        let first = shell.switcher.epoch;
        assert!(shell.switcher.reading);
        let _ = shell.switcher_command(Command::SwitchState);
        assert!(shell.switcher.reread);
        assert_eq!(shell.switcher.epoch, first);
        let _ = shell.switcher_message(SwitcherMessage::Loaded(
            first,
            Ok(SwitcherSnapshot {
                revision: 1,
                active: true,
                selected: 0,
                entries: Vec::new(),
            }),
        ));
        assert!(!shell.switcher.active);
        assert!(shell.switcher.reading);
        assert!(shell.switcher.epoch > first);
        let newest = shell.switcher.epoch;
        let _ = shell.switcher_message(SwitcherMessage::Loaded(
            newest,
            Ok(SwitcherSnapshot {
                revision: 2,
                active: false,
                selected: 0,
                entries: Vec::new(),
            }),
        ));
        assert!(!shell.switcher.active && !shell.switcher.reading);
        let _ = shell.switcher_message(SwitcherMessage::Loaded(
            first,
            Ok(SwitcherSnapshot {
                revision: 1,
                active: true,
                selected: 0,
                entries: Vec::new(),
            }),
        ));
        assert!(!shell.switcher.active);
        assert!(shell.switcher.window.is_none());
    }
    // The overlay window always contains the current selection past ten apps.
    {
        for count in 1..=64 {
            for selected in 0..count {
                let range = visible_entries(count, selected);
                assert!(range.contains(&selected));
                assert!(range.len() <= 10);
                assert!(range.end <= count);
            }
        }
        assert_eq!(visible_entries(64, 63), 54..64);
        assert_eq!(visible_entries(0, 0), 0..0);
    }
    // Completed compositor state closes overlay without second focus request.
    {
        let mut shell = shell();
        shell.switcher.active = true;
        shell.switcher.window = Some(window::Id::unique());
        shell.switcher.epoch = 3;
        shell.switcher.reading = true;
        let _ = shell.switcher_message(SwitcherMessage::Loaded(
            3,
            Ok(SwitcherSnapshot {
                revision: 8,
                active: false,
                selected: 0,
                entries: Vec::new(),
            }),
        ));
        assert!(shell.switcher.window.is_none());
        assert!(!shell.switcher.active);
        assert_eq!(shell.switcher.revision, 8);
        assert!(!shell.switcher.reading);
        let _ = shell.switcher_message(SwitcherMessage::Pick(7, 0));
        assert!(!shell.switcher.reading);
    }
}
