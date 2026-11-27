use super::{Message as Parent, Page, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_shell::{
    control_grid::{Detail, Edit, Grid},
    control_layout::{Layout, Shape, row_height},
    control_tile::{self, Info, ValueControl},
    settings::{Change, QuickAction},
    system_controls::{SystemControls, SystemSnapshot},
};
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    enz_text,
    radio_group::{Orientation, RadioChoice, RadioGroupSize, enz_radio_group_keyed},
    toolbar_icon::enz_toolbar_icon,
};
use iced::{
    Element, Length, Task,
    widget::{column, container, responsive, row, space},
};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct State {
    system: SystemSnapshot,
    reading: bool,
    loaded: bool,
    // Which controls this system has changes rarely: it is read on entering
    // the page and then every 10 s, not on every 1 s tick.
    read_at: Option<std::time::Instant>,
    selected: Option<QuickAction>,
    details: Option<QuickAction>,
    details_open: bool,
    pub focus_pending: Option<String>,
}
impl State {
    pub(super) fn navigation(&mut self) {
        self.details = None;
        self.details_open = false;
        self.focus_pending = None;
    }
}
#[derive(Clone, Debug)]
pub(super) enum Message {
    Snapshot(SystemSnapshot),
    Grid(Edit),
    Details(Option<QuickAction>),
    Shape(QuickAction, Shape, Arc<Layout>),
    Columns(u8, Arc<Layout>),
    Move(QuickAction, i32, Arc<Layout>),
    Visible(QuickAction, bool, u32),
    Reset(Arc<Layout>),
}
static SHAPES: [RadioChoice<'static, Shape>; 4] = [
    RadioChoice::new("compact", Shape::Compact, "Compact"),
    RadioChoice::new("pill", Shape::Pill, "2 wide"),
    RadioChoice::new("triple", Shape::Triple, "3 wide"),
    RadioChoice::new("wide", Shape::Wide, "Full"),
];

impl SettingsApp {
    fn control_neighbours(
        &self,
        action: QuickAction,
    ) -> (Option<QuickAction>, Option<QuickAction>) {
        let visible: Vec<_> = self
            .settings
            .controls
            .order()
            .iter()
            .copied()
            .filter(|&entry| self.settings.shows(entry) && self.controls.system.available(entry))
            .collect();
        let Some(index) = visible.iter().position(|entry| *entry == action) else {
            return (None, None);
        };
        (
            index.checked_sub(1).map(|i| visible[i]),
            visible.get(index + 1).copied(),
        )
    }
    pub(super) fn read_controls(&mut self) -> Task<Parent> {
        if self.page != Page::Shell || self.controls.reading || self.closing {
            return Task::none();
        }
        if self.controls.loaded
            && self
                .controls
                .read_at
                .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(10))
        {
            return Task::none();
        }
        self.controls.read_at = Some(std::time::Instant::now());
        self.controls.reading = true;
        let system = SystemControls::from_environment();
        Task::perform(async move { system.snapshot().await }, |snapshot| {
            Parent::Controls(Message::Snapshot(snapshot))
        })
    }
    pub(super) fn controls_message(&mut self, message: Message) -> Task<Parent> {
        if let Message::Snapshot(system) = message {
            self.controls.system = system;
            self.controls.reading = false;
            self.controls.loaded = true;
            return Task::none();
        }
        if self.closing || self.page != Page::Shell || !self.filter.trim().is_empty() {
            return Task::none();
        }
        let (expected, value, focus) = match message {
            Message::Grid(Edit::Select(action)) => {
                self.controls.selected = Some(action);
                self.highlighted =
                    enzui_shell::settings_catalog::find(&format!("controls/{}", action.key()))
                        .map(|entry| entry.route);
                self.controls.focus_pending = Some("controls-size".into());
                return Task::none();
            }
            Message::Details(action) => {
                if action.is_some_and(|action| {
                    self.controls.system.available(action)
                        && !control_tile::has_disclosure(self.settings.controls.shape(action))
                }) {
                    return Task::none();
                }
                let previous = self.controls.details;
                self.controls.details_open =
                    action.is_some() && !(previous == action && self.controls.details_open);
                if action.is_some() {
                    self.controls.details = action;
                }
                if action.is_none() {
                    self.controls.focus_pending =
                        previous.map(|action| format!("control-{}-details", action.key()));
                }
                return Task::none();
            }
            Message::Grid(Edit::Commit { expected, value }) => (expected, value, None),
            Message::Columns(count, expected) => {
                // The slider reports every move; only a new count is a change.
                if count == expected.column_count() {
                    return Task::none();
                }
                let mut value = (*expected).clone();
                if value.set_column_count(count).is_err() {
                    return Task::none();
                }
                (expected, Arc::new(value), Some("controls-columns".into()))
            }
            Message::Shape(action, shape, expected) => {
                let mut value = (*expected).clone();
                value.set_shape(action, shape);
                (expected, Arc::new(value), Some("controls-size".to_owned()))
            }
            Message::Move(action, delta, expected) if delta == 1 || delta == -1 => {
                let mut value = (*expected).clone();
                let (previous, next) = self.control_neighbours(action);
                let Some(neighbour) = (if delta < 0 { previous } else { next }) else {
                    return Task::none();
                };
                let before = if delta < 0 {
                    Some(neighbour)
                } else {
                    let index = value
                        .order()
                        .iter()
                        .position(|entry| *entry == neighbour)
                        .expect("the action belongs to the normalized controls catalog");
                    value.order().get(index + 1).copied()
                };
                value.move_before(action, before);
                (
                    expected,
                    Arc::new(value),
                    Some(format!("controls-move-{}-{delta}", action.key())),
                )
            }
            Message::Visible(action, shown, expected)
                if expected == self.settings.quick_actions
                    && (!shown || self.controls.system.available(action)) =>
            {
                return self.update(Parent::Change(Change::QuickAction(action, shown)));
            }
            Message::Reset(expected) => (expected, Arc::default(), None),
            _ => return Task::none(),
        };
        if expected != self.settings.controls {
            return Task::none();
        }
        self.controls.focus_pending = focus;
        self.update(Parent::Change(Change::ControlsLayout { expected, value }))
    }
    pub(super) fn control_editor(&self) -> Element<'_, Parent> {
        let system = &self.controls.system;
        let theme = self.tokens();
        let p = theme.palette.clone();
        let font = f32::from(self.settings.metrics.text);
        let height = row_height(font);
        let mut items = Vec::new();
        let mut unavailable = column![].spacing(2);
        let mut hidden = column![].spacing(2);
        let (mut hidden_count, mut unavailable_count) = (0_usize, 0_usize);
        for action in QuickAction::ALL {
            let route = enzui_shell::settings_catalog::find(&format!("controls/{}", action.key()))
                .expect("the action belongs to the normalized controls catalog")
                .route;
            let visible = self.settings.shows(action) && system.available(action);
            if visible {
                let info = Info::read(
                    action,
                    &self.settings,
                    system,
                    self.settings.tiling.unwrap_or(false),
                    true,
                );
                let expanded_value = action == QuickAction::Brightness
                    && control_tile::inline_value(self.settings.controls.shape(action));
                let shape = self.settings.controls.shape(action);
                let disclosure =
                    info.disclosure && control_tile::has_disclosure(shape) && !expanded_value;
                let theme = theme.clone();
                let value = match action {
                    QuickAction::Volume => system.audio.as_ref().map(|v| v.percent),
                    QuickAction::Brightness => system.brightness.as_ref().map(|v| v.percent),
                    _ => None,
                };
                let tile = responsive(move |size| {
                    control_tile::view(
                        info.clone(),
                        shape,
                        theme.clone(),
                        Parent::Controls(Message::Grid(Edit::Select(action))),
                        disclosure.then_some(Parent::Controls(Message::Details(Some(action)))),
                        value.map(|value| ValueControl {
                            value,
                            minimum: if action == QuickAction::Brightness {
                                5
                            } else {
                                0
                            },
                            change: Box::new(move |_| {
                                Parent::Controls(Message::Grid(Edit::Select(action)))
                            }),
                        }),
                        true,
                        size.width,
                        size.height,
                        font,
                        self.controls.details == Some(action) && self.controls.details_open,
                    )
                });
                items.push((action, disclosure, self.mark_control_anchor(route, tile)));
            } else {
                let available = system.available(action);
                let info = Info::read(action, &self.settings, system, false, true);
                let design: Element<'_, Parent> = if action == QuickAction::Bluetooth {
                    enz_toolbar_icon("eye", Parent::Controls(Message::Details(Some(action))))
                        .id("control-bluetooth-design".into())
                        .aria_label(tr("shell-preview-bluetooth-device-design"))
                        .tooltip(tr("shell-design-preview-no-connection"))
                        .enabled(!self.closing)
                        .into_element()
                } else {
                    space().width(0).into()
                };
                let mut entry = crate::settings_app::settings_row(action.label())
                    .icon(info.icon)
                    .subtitle(if available {
                        String::new()
                    } else {
                        info.detail
                    })
                    .enabled(available && !self.closing);
                entry = if available {
                    entry.control(
                        enz_button(
                            tr("shell-add-2"),
                            Parent::Controls(Message::Visible(
                                action,
                                true,
                                self.settings.quick_actions,
                            )),
                        )
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .icon("plus")
                        .id(format!("controls-add-{}", action.key()).into())
                        .enabled(!self.closing)
                        .into_element(),
                    )
                } else {
                    entry.control(design)
                };
                let entry: Element<'_, Parent> = entry.into_element();
                let entry = self.mark_control_anchor(
                    route,
                    enzui_widgets::keyed_column::keyed_column(vec![(
                        format!("{}:{}", action.key(), self.settings.quick_actions),
                        entry,
                    )]),
                );
                if available {
                    hidden_count += 1;
                    hidden = hidden.push(entry);
                } else {
                    unavailable_count += 1;
                    unavailable = unavailable.push(entry);
                    if self.controls.details_open && self.controls.details == Some(action) {
                        unavailable = unavailable.push(
                            container(control_tile::detail_preview(action, theme.clone()))
                                .height(control_tile::detail_height(action)),
                        );
                    }
                }
            }
        }
        let count = items.len();
        let selected = self
            .highlighted
            .and_then(|route| {
                QuickAction::ALL
                    .into_iter()
                    .find(|action| route == format!("controls/{}", action.key()))
            })
            .or(self.controls.selected);
        let grid = Grid::new(
            std::sync::Arc::clone(&self.settings.controls),
            items,
            height,
            &theme,
            !self.closing,
            |edit| Parent::Controls(Message::Grid(edit)),
        )
        .selected(selected)
        .detail(self.controls.details.map(|action| {
            Detail {
                action,
                open: self.controls.details_open,
                height: control_tile::detail_height(action),
                header_color: if Info::read(
                    action,
                    &self.settings,
                    system,
                    self.settings.tiling.unwrap_or(false),
                    true,
                )
                .active
                {
                    p.accent
                } else {
                    p.surface_alt
                },
                content: control_tile::detail_preview(action, theme.clone()),
            }
        }));
        let columns = self.settings.controls.column_count();
        let controls = std::sync::Arc::clone(&self.settings.controls);
        let column_choices =
            crate::settings_app::settings_row(tr("shell-columns-in-control-center"))
                .icon("columns-3")
                .control(
                    enzui_widgets::stepper::enz_stepper(i32::from(columns), move |value| {
                        Parent::Controls(Message::Columns(
                            value.clamp(2, 6) as u8,
                            std::sync::Arc::clone(&controls),
                        ))
                    })
                    .min(2)
                    .max(6)
                    .value_text(columns.to_string())
                    .id("controls-columns")
                    .enabled(!self.closing)
                    .into_element(),
                )
                .into_element();
        // The editable layout is drawn as the Control Center panel itself,
        // inside the same preview frame the other pictures use.
        let panel = container(
            container(Element::from(grid))
                .width(enzui_shell::control_layout::grid_width_for(font, columns)),
        )
        .padding(14)
        .style(move |_| container::Style {
            background: Some(p.surface.into()),
            border: iced::Border {
                radius: 18.0.into(),
                width: 1.0,
                color: p.border,
            },
            shadow: iced::Shadow {
                color: iced::Color {
                    a: 0.12,
                    ..p.shadow
                },
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            ..Default::default()
        });
        let mut body = column![
            column_choices,
            self.preview_frame(
                column![
                    enz_text(tr(
                        "shell-drag-to-arrange-pull-the-round-edge-handle-to-resize-preview-controls-do-not"
                    ))
                    .size_px(self.text_px(12.0))
                    .muted_on(p.background)
                    .into_element(),
                    container(panel).center_x(Length::Fill).padding([6, 0]),
                ]
                .spacing(10)
            ),
        ]
        .spacing(12);
        if count == 0 {
            body = body.push(enz_text(tr("shell-add-an-available-control-below")).into_element());
        }
        if let Some(action) = selected
            && self.settings.shows(action)
            && system.available(action)
        {
            let base = std::sync::Arc::clone(&self.settings.controls);
            let shape = base.shape(action);
            let choices = enz_radio_group_keyed(
                tr("shell-control-size"),
                &SHAPES,
                Some(&shape),
                move |shape| {
                    Parent::Controls(Message::Shape(action, *shape, std::sync::Arc::clone(&base)))
                },
            )
            .label_visible(false)
            .size(RadioGroupSize::Sm)
            .spacing(10.0)
            .orientation(Orientation::Horizontal)
            .id("controls-size".into())
            .enabled(!self.closing && action != QuickAction::Volume)
            .into_element();
            let (previous, next) = self.control_neighbours(action);
            let tools = row![
                enz_text(action.label())
                    .size_px(self.text_px(13.0))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .width(Length::Fill)
                    .into_element(),
                enz_toolbar_icon(
                    "arrow-left",
                    Parent::Controls(Message::Move(
                        action,
                        -1,
                        std::sync::Arc::clone(&self.settings.controls)
                    ))
                )
                .id(format!("controls-move-{}--1", action.key()).into())
                .aria_label(enzui_shell::tr!("shell-a11y-move-earlier", "arg0" => enzui_shell::i18n::argument(&(action.label()))))
                .tooltip(tr("shell-move-earlier"))
                .enabled(previous.is_some() && !self.closing)
                .into_element(),
                enz_toolbar_icon(
                    "arrow-right",
                    Parent::Controls(Message::Move(
                        action,
                        1,
                        std::sync::Arc::clone(&self.settings.controls)
                    ))
                )
                .id(format!("controls-move-{}-1", action.key()).into())
                .aria_label(enzui_shell::tr!("shell-a11y-move-later", "arg0" => enzui_shell::i18n::argument(&(action.label()))))
                .tooltip(tr("shell-move-later"))
                .enabled(next.is_some() && !self.closing)
                .into_element(),
                enz_toolbar_icon(
                    "eye-off",
                    Parent::Controls(Message::Visible(action, false, self.settings.quick_actions))
                )
                .id(format!("controls-hide-{}", action.key()).into())
                .aria_label(enzui_shell::tr!("shell-a11y-hide", "arg0" => enzui_shell::i18n::argument(&(action.label()))))
                .tooltip(tr("shell-hide-control"))
                .enabled(!self.closing)
                .into_element()
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center);
            let inspector = enzui_widgets::keyed_column::keyed_column(vec![(
                format!(
                    "{}:{:?}:{}",
                    action.key(),
                    self.settings.controls,
                    self.settings.quick_actions
                ),
                column![tools, choices].spacing(8).into(),
            )]);
            body = body.push(
                container(container(inspector).padding(12).width(360).style(move |_| {
                    container::Style {
                        background: Some(p.surface_alt.into()),
                        border: iced::Border {
                            radius: 12.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                }))
                .center_x(Length::Fill),
            );
        }
        if !self.controls.loaded {
            body = body.push(
                enz_text(tr("shell-checking-available-controls"))
                    .muted_on(p.background)
                    .into_element(),
            );
        }
        let heading = |label: &'static str| {
            container(
                enz_text(label)
                    .size_px(self.text_px(12.0))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .muted_on(p.surface)
                    .into_element(),
            )
            .padding(iced::Padding::ZERO.left(10.0).top(6.0))
        };
        if hidden_count > 0 {
            body = body.push(heading(tr("shell-hidden-controls"))).push(hidden);
        }
        if unavailable_count > 0 {
            body = body
                .push(heading(tr("shell-not-available-on-this-computer")))
                .push(unavailable);
        }
        body.push(iced::widget::row![
            iced::widget::space::horizontal(),
            enz_button(
                tr("shell-reset-layout"),
                Parent::Controls(Message::Reset(std::sync::Arc::clone(
                    &self.settings.controls
                ))),
            )
            .kind(ButtonKind::Quiet)
            .icon("rotate-ccw")
            .id("controls-reset".into())
            .enabled(!self.closing)
            .into_element(),
        ])
        .into()
    }
}
