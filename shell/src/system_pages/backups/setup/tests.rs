use super::*;

fn planned() -> Status {
    Status {
        state: "planned".into(),
        attempts: 0,
        digest: "a".repeat(64),
        retryable: false,
        cancellable: true,
    }
}
#[test]
fn setup_behaviour() {
    // Enrollment requires review and exact password confirmation for new repository.
    {
        let mut setup = Setup {
            open: true,
            mode: Some(Mode::Initialize),
            password: Some(SecretText::new("synthetic private".into())),
            repeat: Some(SecretText::new("different".into())),
            ..Setup::default()
        };
        assert!(!setup.input_ready());
        setup.status = Some(planned());
        assert!(!setup.input_ready());
        setup.repeat = Some(SecretText::new("synthetic private".into()));
        assert!(setup.input_ready());
        setup.busy = true;
        assert!(!setup.input_ready());
        setup.busy = false;
        setup.error = Some("status unknown".into());
        assert!(!setup.input_ready());
        setup.error = None;
        setup.mode = Some(Mode::Adopt);
        setup.repeat = None;
        assert!(setup.input_ready());
        setup.clear_secrets();
        assert!(setup.password.is_none());
        assert!(setup.repeat.is_none());
        assert!(!setup.input_ready());
    }
    // Settings navigation clears secrets and rejects queued input.
    {
        for navigation in [
            Parent::Filter("password".into()),
            Parent::Reveal("appearance/theme"),
            Parent::Section("apps/defaults"),
            Parent::Page(Category::Apps),
            Parent::Close,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _) = SettingsApp::new(
                enzui_shell::settings::Store::at(directory.path().join("settings.json")).unwrap(),
            );
            app.page = Category::Backups;
            app.system.backups.setup = Setup {
                open: true,
                mode: Some(Mode::Initialize),
                status: Some(planned()),
                password: Some(SecretText::new("synthetic private".into())),
                repeat: Some(SecretText::new("synthetic private".into())),
                revealed: true,
                repeat_revealed: true,
                next_poll: Some(Instant::now()),
                ..Setup::default()
            };
            let _ = app.update(navigation);
            let _ = app.backup_setup_message(SetupMessage::Password(SecretText::new(
                "queued private".into(),
            )));
            let _ = app.backup_setup_message(SetupMessage::Repeat(SecretText::new(
                "queued private".into(),
            )));
            let _ = app.backup_setup_message(SetupMessage::Reveal);
            let setup = &app.system.backups.setup;
            assert!(!setup.open);
            assert!(setup.password.is_none());
            assert!(setup.repeat.is_none());
            assert!(!setup.revealed && !setup.repeat_revealed);
            assert!(setup.next_poll.is_none());
            assert_eq!(setup.status.as_ref().unwrap().state, "planned");
        }
    }
}
