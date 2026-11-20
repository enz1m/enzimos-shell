use super::*;
use enzui_shell::controllers::{Controller, Input, axis_name, button_name};
use enzui_shell::i18n::tr;
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use iced::widget::{column, container, row};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub(super) struct Test {
    pub id: String,
    pub node: PathBuf,
    generation: u64,
    pressed: BTreeMap<u16, bool>,
    axes: BTreeMap<u16, (i32, i32, i32)>,
    error: Option<String>,
    vibration_target: Result<enzui_shell::controller_vibration::Target, String>,
    vibration_epoch: u64,
    vibration_running: bool,
    vibration_stopping: bool,
    vibration_result: Option<Result<(), String>>,
    vibration_work: Option<iced::task::Handle>,
}

impl Test {
    pub(super) fn new(id: String, node: PathBuf) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let vibration_target = enzui_shell::controller_vibration::Target::capture(&node);
        Self {
            id,
            node,
            generation: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            pressed: BTreeMap::new(),
            axes: BTreeMap::new(),
            error: None,
            vibration_target,
            vibration_epoch: 0,
            vibration_running: false,
            vibration_stopping: false,
            vibration_result: None,
            vibration_work: None,
        }
    }
    pub(super) fn stop_vibration(&mut self) {
        if let Some(work) = self.vibration_work.take() {
            work.abort();
        }
        self.vibration_epoch = self.vibration_epoch.saturating_add(1);
        self.vibration_running = false;
        self.vibration_stopping = false;
    }
    pub(super) fn apply(&mut self, generation: u64, input: Input) {
        if self.generation != generation {
            return;
        }
        match input {
            Input::Button { code, pressed } => {
                if self.pressed.len() < 64 || self.pressed.contains_key(&code) {
                    self.pressed.insert(code, pressed);
                }
            }
            Input::Axis {
                code,
                value,
                min,
                max,
            } => {
                if self.axes.len() < 16 || self.axes.contains_key(&code) {
                    self.axes.insert(code, (value, min, max));
                }
            }
            Input::Gone(error) => {
                self.stop_vibration();
                self.error = Some(error);
            }
        }
    }
}

impl Drop for Test {
    fn drop(&mut self) {
        self.stop_vibration();
    }
}

impl SettingsApp {
    fn controller_context_active(&self) -> bool {
        self.page == Category::Controllers
            && self.filter.trim().is_empty()
            && !self.closing
            && self.window.is_some()
    }

    pub(super) fn can_test_controllers(&self) -> bool {
        self.controller_context_active()
            && (enzui_shell::desktop_session::mode()
                != enzui_shell::desktop_session::Mode::Installed
                || self.system.controller_seat == enzui_shell::session_activity::State::Active)
    }

    pub(crate) fn controller_context_changed(&mut self) {
        if !self.controller_context_active() {
            self.system.controller_test = None;
            self.system.controller_seat = Default::default();
            self.system.controller_notice = None;
        } else if !self.can_test_controllers() {
            self.system.controller_test = None;
        }
        if !self.controller_seat_needed() {
            self.system.controller_seat = Default::default();
        }
        if self.system.controller_seat != enzui_shell::session_activity::State::Active
            && let Some(test) = &mut self.system.controller_test
        {
            test.stop_vibration();
        }
    }

    fn controller_seat_needed(&self) -> bool {
        enzui_shell::desktop_session::mode() == enzui_shell::desktop_session::Mode::Installed
            || self.system.controller_test.as_ref().is_some_and(|test| {
                self.system.controllers.as_ref().is_some_and(|list| {
                    list.iter().any(|controller| {
                        controller.id == test.id
                            && controller.node == test.node
                            && controller.rumble
                    })
                })
            })
    }

    // Watches controllers only while their page is open, and reads one
    // only while it is being tested.
    pub(crate) fn controller_subscription(&self) -> iced::Subscription<Parent> {
        if !self.controller_context_active() {
            return iced::Subscription::none();
        }
        let list = enzui_shell::controllers::subscription()
            .map(|list| Parent::System(Message::Controllers(list)));
        let seat = if self.controller_seat_needed() {
            enzui_shell::session_activity::subscription()
                .map(|state| Parent::System(Message::ControllerSeat(state)))
        } else {
            iced::Subscription::none()
        };
        let input = match &self.system.controller_test {
            Some(test) if test.error.is_none() && self.can_test_controllers() => {
                let generation = test.generation;
                iced::Subscription::run_with(
                    (generation, test.node.clone()),
                    |(_, node): &(u64, PathBuf)| enzui_shell::controllers::test(node.clone()),
                )
                .with(generation)
                .map(|(generation, input)| {
                    Parent::System(Message::ControllerInput(generation, input))
                })
            }
            _ => iced::Subscription::none(),
        };
        iced::Subscription::batch([list, seat, input])
    }

    fn can_vibrate_controller(&self) -> bool {
        self.controller_context_active()
            && self.system.controller_seat == enzui_shell::session_activity::State::Active
            && self.controls_live()
    }

    pub(super) fn controller_vibrate(&mut self, generation: u64) -> Task<Parent> {
        if !self.can_vibrate_controller() {
            return Task::none();
        }
        let Some(test) = self.system.controller_test.as_mut().filter(|test| {
            test.generation == generation && test.error.is_none() && !test.vibration_running
        }) else {
            return Task::none();
        };
        let supported = self.system.controllers.as_ref().is_some_and(|list| {
            list.iter().any(|controller| {
                controller.id == test.id
                    && controller.node == test.node
                    && controller.rumble
                    && controller.accessible
            })
        });
        if !supported {
            return Task::none();
        }
        let target = match test.vibration_target.clone() {
            Ok(target) => target,
            Err(error) => {
                test.vibration_result = Some(Err(error));
                return Task::none();
            }
        };
        let Some(epoch) = test.vibration_epoch.checked_add(1) else {
            return Task::none();
        };
        test.vibration_epoch = epoch;
        test.vibration_running = true;
        test.vibration_stopping = false;
        test.vibration_result = None;
        let (task, handle) = Task::run(
            enzui_shell::controller_vibration::run(target),
            move |event| Parent::System(Message::ControllerVibration(generation, epoch, event)),
        )
        .abortable();
        test.vibration_work = Some(handle);
        task
    }

    pub(super) fn controller_vibration_event(
        &mut self,
        generation: u64,
        epoch: u64,
        event: enzui_shell::controller_vibration::Event,
    ) {
        if !self.controller_context_active()
            || self.system.controller_seat != enzui_shell::session_activity::State::Active
        {
            return;
        }
        if let Some(test) = self.system.controller_test.as_mut().filter(|test| {
            test.generation == generation && test.vibration_epoch == epoch && test.vibration_running
        }) {
            match event {
                enzui_shell::controller_vibration::Event::Stopping => {
                    test.vibration_stopping = true;
                }
                enzui_shell::controller_vibration::Event::Finished(result) => {
                    test.vibration_work = None;
                    test.vibration_running = false;
                    test.vibration_stopping = false;
                    test.vibration_result = Some(result);
                }
            }
        }
    }

    fn test_panel<'a>(&'a self, test: &'a Test) -> Element<'a, Parent> {
        let palette = self.tokens().palette;
        let (accent, alt, on_accent, ink) = (
            palette.accent,
            palette.surface_alt,
            palette.on_accent(),
            palette.text,
        );
        let mut content = column![].spacing(10);
        if let Some(error) = &test.error {
            content = content.push(self.caption(error.as_str()));
        } else if test.pressed.is_empty() && test.axes.is_empty() {
            content = content.push(self.caption(tr("shell-press-a-button-or-move-a-stick")));
        }
        let mut buttons = row![].spacing(6);
        for (code, pressed) in test.pressed.iter().take(20) {
            let pressed = *pressed;
            buttons = buttons.push(
                container(
                    enz_text(button_name(*code))
                        .size(TextSize::Caption)
                        .into_element(),
                )
                .padding([3, 8])
                .style(move |_| iced::widget::container::Style {
                    background: Some(if pressed { accent } else { alt }.into()),
                    text_color: Some(if pressed { on_accent } else { ink }),
                    border: iced::Border {
                        radius: 6.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }
        if !test.pressed.is_empty() {
            content = content.push(iced::widget::scrollable(buttons).direction(
                iced::widget::scrollable::Direction::Horizontal(Default::default()),
            ));
        }
        for (code, (value, min, max)) in &test.axes {
            let at = axis_fraction(*value, *min, *max);
            let bar = container(row![
                container(iced::widget::Space::new())
                    .width(iced::Length::FillPortion((at * 1000.0) as u16 + 1))
                    .height(6)
                    .style(move |_| iced::widget::container::Style {
                        background: Some(accent.into()),
                        border: iced::Border {
                            radius: 3.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                iced::widget::Space::new()
                    .width(iced::Length::FillPortion(((1.0 - at) * 1000.0) as u16 + 1)),
            ])
            .width(iced::Length::Fill)
            .style(move |_| iced::widget::container::Style {
                background: Some(alt.into()),
                border: iced::Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
            content = content.push(
                row![
                    enz_text(axis_name(*code))
                        .size(TextSize::Caption)
                        .width(iced::Length::Fixed(110.0))
                        .into_element(),
                    bar
                ]
                .spacing(10)
                .align_y(iced::Alignment::Center),
            );
        }
        let rumble = self.system.controllers.as_ref().is_some_and(|list| {
            list.iter().any(|controller| {
                controller.id == test.id && controller.node == test.node && controller.rumble
            })
        });
        if rumble {
            content = content.push(
                enz_button(
                    if test.vibration_stopping {
                        tr("shell-stopping-vibration")
                    } else if test.vibration_running {
                        tr("shell-testing-vibration")
                    } else {
                        tr("shell-test-vibration")
                    },
                    Parent::System(Message::VibrateController(test.generation)),
                )
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .id(format!("controller-vibrate-{}", test.generation).into())
                .enabled(
                    self.can_vibrate_controller()
                        && test.error.is_none()
                        && !test.vibration_running,
                )
                .into_element(),
            );
            content = content.push(self.caption(
                if self.system.controller_seat == enzui_shell::session_activity::State::Active {
                    tr("shell-a-brief-low-strength-pulse-on-this-controller-only")
                } else {
                    tr("shell-vibration-requires-an-unlocked-active-local-session")
                },
            ));
            if test.vibration_stopping {
                content = content.push(self.caption(tr("shell-waiting-for-the-controller-driver-to-stop-another-vibration-cannot-start-yet")));
            }
            if let Some(result) = &test.vibration_result {
                content = content.push(self.caption(match result {
                    Ok(()) => tr("shell-vibration-request-sent").to_owned(),
                    Err(error) => error.clone(),
                }));
            }
        }
        container(content)
            .padding(iced::Padding {
                top: 4.0,
                right: 12.0,
                bottom: 10.0,
                left: 44.0,
            })
            .into()
    }

    fn controller_row<'a>(
        &'a self,
        index: usize,
        controller: &'a Controller,
    ) -> Element<'a, Parent> {
        let testing = self
            .system
            .controller_test
            .as_ref()
            .is_some_and(|t| t.id == controller.id);
        let mut details = vec![controller.transport.label().to_owned()];
        if let Some(battery) = controller.battery {
            details.push(battery.label());
        }
        details.push(if controller.rumble {
            tr("shell-rumble-supported").to_owned()
        } else {
            tr("shell-no-rumble").to_owned()
        });
        let active = self.can_test_controllers();
        let test = enz_button(
            if testing {
                tr("shell-stop-test")
            } else {
                tr("shell-test")
            },
            Parent::System(Message::TestController(if testing {
                None
            } else {
                Some(controller.id.clone())
            })),
        )
        .kind(ButtonKind::Secondary)
        .size(ButtonSize::Sm)
        .id(format!("controller-test-{index}").into())
        .enabled(controller.accessible && active)
        .into_element();
        let mut column = column![
            crate::settings_app::settings_row(controller.name.clone())
                .subtitle(if !controller.accessible { tr("shell-this-account-cannot-use-it-it-belongs-to-the-active-session-on-this-seat").to_owned() } else if !active { tr("shell-controller-testing-requires-an-unlocked-active-session").to_owned() } else { details.join(" · ") })
                .icon("gamepad-2")
                .control(test)
                .into_element()
        ];
        if testing && let Some(test) = &self.system.controller_test {
            column = column.push(self.test_panel(test));
        }
        column.into()
    }

    pub(super) fn controllers_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let route = "controllers/list";
        let mut rows = column![].spacing(2);
        match &self.system.controllers {
            None => rows = rows.push(self.mark_setting(route, self.loading_row())),
            Some(list) if list.is_empty() => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "gamepad-2",
                        tr("shell-no-controllers-connected"),
                        tr("shell-plug-one-in-with-usb-or-pair-a-wireless-controller-in-bluetooth"),
                    ),
                ));
            }
            Some(list) => {
                for (index, controller) in list.iter().enumerate() {
                    let row = self.controller_row(index, controller);
                    rows = rows.push(if index == 0 {
                        self.mark_setting(route, row)
                    } else {
                        row
                    });
                }
            }
        }
        if let Some(notice) = self.system.controller_notice {
            rows = rows.push(container(self.caption(notice)).padding([10, 16]));
        }
        rows = rows.push(
            crate::settings_app::settings_row(tr("shell-pair-a-wireless-controller"))
                .subtitle(tr(
                    "shell-put-the-controller-in-pairing-mode-then-add-it-in-bluetooth",
                ))
                .icon("bluetooth")
                .action("", Parent::Page(Category::Bluetooth))
                .into_element(),
        );
        body.push(
            self.section_card_for(route, rows)
                .title(tr("shell-connected-controllers"))
                .description(tr("shell-test-a-controller-s-buttons-and-sticks-games-and-steam-input-keep-their-own"))
                .into_element(),
        )
        .push(
            self.section_card_for(
                "controllers/notify",
                self.switch_setting("controllers/notify", self.settings.controller_notifications, |on| Parent::Change(enzui_shell::settings::Change::ControllerNotifications(on))),
            )
            .title(tr("shell-notifications"))
            .into_element(),
        )
    }
}

fn axis_fraction(value: i32, min: i32, max: i32) -> f32 {
    let span = (i64::from(max) - i64::from(min)).max(1) as f32;
    ((i64::from(value) - i64::from(min)) as f32 / span).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controllers_behaviour() {
        // Vibration requires controller context and a verified active seat.
        {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _) = SettingsApp::new(
                crate::settings_app::Store::at(directory.path().join("settings.json")).unwrap(),
            );
            app.page = Category::Controllers;
            app.window = Some(iced::window::Id::unique());
            assert!(!app.can_vibrate_controller());
            app.system.controller_seat = enzui_shell::session_activity::State::Active;
            assert!(app.can_vibrate_controller());
            app.filter = "search".into();
            assert!(!app.can_vibrate_controller());
            app.filter.clear();
            app.closing = true;
            assert!(!app.can_vibrate_controller());
        }
        // A stop request stays pending until the worker is reaped.
        {
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _) = SettingsApp::new(
                crate::settings_app::Store::at(directory.path().join("settings.json")).unwrap(),
            );
            app.page = Category::Controllers;
            app.window = Some(iced::window::Id::unique());
            app.system.controller_seat = enzui_shell::session_activity::State::Active;
            let mut test = Test::new("fixture".into(), "/dev/null".into());
            let generation = test.generation;
            test.vibration_epoch = 1;
            test.vibration_running = true;
            app.system.controller_test = Some(test);
            app.controller_vibration_event(
                generation,
                1,
                enzui_shell::controller_vibration::Event::Stopping,
            );
            let test = app.system.controller_test.as_ref().unwrap();
            assert!(
                test.vibration_running
                    && test.vibration_stopping
                    && test.vibration_result.is_none()
            );
            app.controller_vibration_event(
                generation,
                1,
                enzui_shell::controller_vibration::Event::Finished(Err("Driver stopped".into())),
            );
            let test = app.system.controller_test.as_ref().unwrap();
            assert!(
                !test.vibration_running
                    && !test.vibration_stopping
                    && test.vibration_result.is_some()
            );
        }
    }
}
