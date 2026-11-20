use super::*;
use crate::accounts::{Session, User};

fn greeter(users: usize, sessions: usize, power: bool) -> Greeter {
    let user = |i: usize| User {
        name: format!("u{i}"),
        display: format!("User {i}"),
    };
    let session = |i: usize| Session {
        id: format!("s{i}"),
        name: format!("Session {i}"),
        exec: "run".into(),
        desktop_names: String::new(),
        icon: None,
        path: Default::default(),
    };
    Greeter {
        users: (0..users).map(user).collect(),
        user: 0,
        sessions: (0..sessions).map(session).collect(),
        session: 0,
        prompt: None,
        notice: None,
        power,
        existing: None,
    }
}

#[test]
fn ui_behaviour() {
    // Greeter can begin and answer pam with an empty response.
    {
        let mut ui = Ui::new("Ann".into(), true);
        assert!(!ui.answer_ready());
        ui.greeter = Some(greeter(1, 1, false));
        assert!(ui.answer_ready());
        ui.greeter.as_mut().unwrap().prompt = Some(("Current password:".into(), true));
        assert!(ui.answer_ready());
    }
    // Wake and retained controls reverse without snapping.
    {
        let mut ui = Ui::new("Ann".into(), true);
        let now = Instant::now();
        ui.last_input = Some(now);
        ui.tick(now, 0.2);
        ui.tick(now, 0.2);
        let waking = ui.focus;
        assert!(waking > 0.0 && waking < 1.0);
        ui.last_input = None;
        ui.tick(now, 0.04);
        assert!(ui.focus > 0.0 && ui.focus < waking);
        ui.local(Message::TogglePower);
        ui.tick(now, 0.12);
        ui.tick(now, 0.12);
        let opening = ui.power_reveal;
        assert!(opening > 0.0 && opening < 1.0);
        ui.local(Message::TogglePower);
        assert!(!ui.targets().contains(&Target::Sleep));
        ui.tick(now, 0.04);
        assert!(ui.power_reveal > 0.0 && ui.power_reveal < opening);
        ui.last_input = None;
        ui.started = now - FOCUS_PULL;
        for _ in 0..10 {
            let moving = ui.focus > 0.0 || ui.power_reveal > 0.0;
            let redraw = ui.tick(now, 0.1);
            if moving {
                assert!(redraw, "settlement must paint the final frame");
            }
        }
        assert!(
            !ui.tick(now, 0.1),
            "settled controls must not keep rendering"
        );
    }
    // Tab reaches every control in order and wraps.
    {
        let mut ui = Ui::new("Ann".into(), true);
        assert_eq!(ui.targets(), [Target::Field, Target::Keyboard]);
        ui.greeter = Some(greeter(2, 2, true));
        assert_eq!(
            ui.targets(),
            [
                Target::Field,
                Target::PreviousUser,
                Target::NextUser,
                Target::Keyboard,
                Target::Session,
                Target::PowerMenu
            ]
        );
        ui.move_focus(false);
        assert_eq!(ui.target, Target::PowerMenu);
        assert!(ui.focus_visible);
        ui.move_focus(true);
        assert_eq!(ui.target, Target::Field);
        ui.greeter = Some(greeter(1, 1, false));
        ui.target = Target::Keyboard;
        ui.move_focus(true);
        assert_eq!(ui.target, Target::Field);
        assert_eq!(Target::Session.message(), Message::NextSession);
    }
    // Power menu focus is contained and selection closes it.
    {
        let mut ui = Ui::new("Ann".into(), true);
        ui.greeter = Some(greeter(1, 1, true));
        assert!(!ui.targets().contains(&Target::Session));
        assert!(!ui.targets().contains(&Target::PreviousUser));
        ui.local(Message::TogglePower);
        assert_eq!(
            ui.targets(),
            [
                Target::PowerMenu,
                Target::Sleep,
                Target::Restart,
                Target::PowerOff
            ]
        );
        assert_eq!(ui.target, Target::Sleep);
        ui.move_focus(false);
        assert_eq!(ui.target, Target::PowerMenu);
        assert_eq!(ui.local(Message::Sleep), Some(Message::Sleep));
        assert!(!ui.power_menu);
    }
    // Keyboard rows scale together and fit small outputs.
    {
        for (width, height) in [
            (320.0, 480.0),
            (480.0, 600.0),
            (800.0, 600.0),
            (1366.0, 768.0),
            (1100.0, 320.0),
        ] {
            let size = OskSize::fitting(iced::Size::new(width, height));
            let letters = (40.0 * 10.0 + 6.0 * 9.0) * size.width;
            let controls = (64.0 + 314.0 + 64.0 + 6.0 * 2.0) * size.width;
            assert!((letters - controls).abs() < 0.01);
            assert!(letters + 20.0 * size.width <= width - 32.0);
            assert!(size.height >= 16.0);
            assert!(244.0 * size.width + 194.0 <= height);
        }
    }
    // On screen keys type the password only while waiting.
    {
        let mut ui = Ui::new("Ann".into(), true);
        assert!(ui.local(Message::ToggleKeyboard).is_none());
        assert!(ui.keyboard);
        ui.local(Message::Osk(Osk::Shift));
        ui.local(Message::Osk(Osk::Text("A".into())));
        assert!(!ui.shift, "shift applies to one key");
        ui.local(Message::Osk(Osk::Text("b".into())));
        ui.local(Message::Osk(Osk::Backspace));
        assert_eq!(ui.password, "A");
        ui.phase = Phase::Checking;
        ui.local(Message::Osk(Osk::Text("c".into())));
        assert_eq!(ui.password, "A");
        assert_eq!(ui.local(Message::Submit), Some(Message::Submit));
        assert_eq!(ui.local(Message::Restart), Some(Message::Restart));
    }
    // Status announces errors before notices and caps lock.
    {
        let mut ui = Ui::new("Ann".into(), true);
        ui.caps_lock = true;
        assert_eq!(ui.status(), "Caps Lock is on");
        ui.greeter = Some(Greeter {
            notice: Some("Your password expires soon".into()),
            ..greeter(1, 1, false)
        });
        assert_eq!(ui.status(), "Your password expires soon");
        ui.error = Some(("Incorrect password. Try again.".into(), Instant::now()));
        assert_eq!(ui.status(), "Incorrect password. Try again.");
    }
}
