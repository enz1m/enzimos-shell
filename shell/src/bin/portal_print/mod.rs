mod parent;

use super::{Message, window};
use enzui_shell::arm_delay::ArmGate;
use enzui_shell::desktop_portal::print_portal::{
    DialogAnswer, DialogRequest, Duplex, Orientation, Pages,
};
use enzui_shell::i18n::tr;
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    combo_box::enz_combo_box,
    text::{TextSize, enz_text},
    text_field::enz_text_field,
};
use iced::widget::{Space, column, container, row, scrollable};
use iced::{Alignment, Element, Length, Task};
use tokio::sync::oneshot;

pub(super) use parent::ParentProtocol;

#[derive(Clone, Debug)]
pub(super) struct Surface {
    connection: wayland_client::Connection,
    surface: wayland_client::protocol::wl_surface::WlSurface,
    toplevel: Option<wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel>,
}

#[derive(Clone, Debug)]
pub(super) enum Edit {
    Printer(usize),
    Copies(String),
    Pages(Pages),
    Range(String),
    Orientation(Orientation),
    Paper(usize),
    Duplex(Duplex),
    Color(bool),
}

pub(super) struct Dialog {
    request: DialogRequest,
    reply: oneshot::Sender<DialogAnswer>,
    parent: Option<parent::Parent>,
    pub(super) gate: ArmGate,
}

impl Dialog {
    pub(super) fn new(request: DialogRequest, reply: oneshot::Sender<DialogAnswer>) -> Self {
        Self {
            request,
            reply,
            parent: None,
            gate: ArmGate::default(),
        }
    }

    pub(super) fn id(&self) -> u64 {
        self.request.id
    }

    pub(super) fn title(&self) -> &str {
        if self.request.title.is_empty() {
            tr("shell-print")
        } else {
            &self.request.title
        }
    }

    pub(super) fn parent_task(&self, id: window::Id) -> Task<Message> {
        if !self.request.parent_window.starts_with("wayland-") {
            return Task::none();
        }
        window::wayland_parent_export(id).then(move |payload| {
            let Some(payload) = payload.and_then(|payload| {
                payload
                    .downcast::<(
                        wayland_client::Connection,
                        wayland_client::protocol::wl_surface::WlSurface,
                        u64,
                    )>()
                    .ok()
            }) else {
                return Task::none();
            };
            let (connection, surface, _) = *payload;
            window::wayland_toplevel(id).map(move |toplevel| {
                let toplevel = toplevel
                    .and_then(|value| value.downcast().ok())
                    .map(|value| *value);
                Message::PrintSurface(
                    id,
                    Surface {
                        connection: connection.clone(),
                        surface: surface.clone(),
                        toplevel,
                    },
                )
            })
        })
    }

    pub(super) fn attach(&mut self, protocol: &mut Option<ParentProtocol>, surface: Surface) {
        self.parent = parent::Parent::attach(
            protocol,
            surface,
            &self.request.parent_window,
            self.request.modal,
        );
    }

    fn can_print(&self) -> bool {
        self.request.selection.as_ref().is_some_and(|choice| {
            choice.valid_copies() && (choice.pages != Pages::Range || choice.valid_range())
        })
    }

    pub(super) fn can_finish(&self) -> bool {
        self.request.selection.is_none() || self.can_print()
    }

    pub(super) fn answer(self) {
        let answer = if self.can_print() {
            self.request
                .selection
                .map(|selection| DialogAnswer::Chosen(Box::new(selection)))
        } else if self.request.selection.is_none() {
            Some(DialogAnswer::Unavailable)
        } else {
            None
        };
        if let Some(answer) = answer {
            let _ = self.reply.send(answer);
        }
    }

    pub(super) fn edit(&mut self, edit: Edit) {
        let Some(choice) = &mut self.request.selection else {
            return;
        };
        match edit {
            Edit::Printer(index) => {
                if let Ok(printers) = &self.request.printers
                    && let Some(printer) = printers.get(index)
                    && choice.printer_index != index
                {
                    choice.choose_printer(index, printer);
                }
            }
            Edit::Copies(value) => choice.copies = value.chars().take(10).collect(),
            Edit::Pages(value) => choice.pages = value,
            Edit::Range(value) => choice.page_ranges = value.chars().take(256).collect(),
            Edit::Orientation(value) => choice.orientation = value,
            Edit::Paper(index) => choice.paper = index,
            Edit::Duplex(value) => choice.duplex = Some(value),
            Edit::Color(value) => choice.color = Some(value),
        }
    }

    pub(super) fn view(
        &self,
        window: window::Id,
        tokens: &enzui_theme::EnzTheme,
    ) -> Element<'_, Message> {
        let header = row![
            enzui_widgets::icon::enz_icon("printer")
                .size(24.0)
                .into_element(),
            enz_text(self.title())
                .size(TextSize::Title3)
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let available = self.request.selection.is_some();
        let footer = row![Space::new().width(Length::Fill),];
        let footer = if available {
            footer
                .push(
                    enz_button(tr("shell-cancel"), Message::Cancel(window))
                        .kind(ButtonKind::Secondary)
                        .id("print-cancel".into())
                        .into_element(),
                )
                .push(
                    enz_button(
                        self.request
                            .accept_label
                            .as_deref()
                            .unwrap_or_else(|| tr("shell-print")),
                        Message::PrintAccept(window),
                    )
                    .enabled(self.gate.armed() && self.can_print())
                    .id("print-submit".into())
                    .into_element(),
                )
        } else {
            footer.push(
                enz_button(tr("shell-close"), Message::PrintAccept(window))
                    .id("print-close".into())
                    .into_element(),
            )
        };
        container(
            column![
                header,
                scrollable(self.fields(window, tokens)).height(Length::Fill),
                footer.spacing(8),
            ]
            .spacing(18),
        )
        .padding(24)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn fields(&self, window: window::Id, tokens: &enzui_theme::EnzTheme) -> Element<'_, Message> {
        let (printers, choice) = match (&self.request.printers, &self.request.selection) {
            (Ok(printers), Some(choice)) => (printers, choice),
            (printers, _) => {
                let label = if printers.is_err() {
                    tr("shell-print-service-unavailable")
                } else {
                    tr("shell-print-no-printers")
                };
                return enz_text(label)
                    .muted_on(tokens.palette.surface)
                    .into_element();
            }
        };
        let printer = &printers[choice.printer_index];
        let mut page_modes = vec![Pages::All];
        if self.request.has_current_page {
            page_modes.push(Pages::Current);
        }
        if self.request.has_selected_pages {
            page_modes.push(Pages::Selection);
        }
        page_modes.push(Pages::Range);
        let page_label = |mode| {
            tr(match mode {
                Pages::All => "shell-print-all-pages",
                Pages::Current => "shell-print-current-page",
                Pages::Selection => "shell-print-selected-pages",
                Pages::Range => "shell-print-page-range",
            })
        };
        let mode_labels = page_modes
            .iter()
            .copied()
            .map(|mode| page_label(mode).into())
            .collect();
        let mode_index = page_modes.iter().position(|mode| *mode == choice.pages);
        let orientation_label = |mode| {
            tr(match mode {
                Orientation::Portrait => "shell-print-portrait",
                Orientation::Landscape => "shell-print-landscape",
                Orientation::ReversePortrait => "shell-print-reverse-portrait",
                Orientation::ReverseLandscape => "shell-print-reverse-landscape",
            })
        };
        let duplex_label = |mode| {
            tr(match mode {
                Duplex::OneSided => "shell-print-one-sided",
                Duplex::LongEdge => "shell-print-long-edge",
                Duplex::ShortEdge => "shell-print-short-edge",
            })
        };
        let copies = enz_text_field("", "", &choice.copies, move |value| {
            Message::PrintEdit(window, Edit::Copies(value))
        })
        .aria_label(tr("shell-print-copies"))
        .id("print-copies".into());
        let copies = if choice.valid_copies() {
            copies
        } else {
            copies
                .error(enzui_shell::tr!("shell-print-copies-error", "max" => printer.max_copies()))
        };
        let mut fields = column![
            field(
                "shell-print-printer",
                combo(
                    window,
                    "shell-print-printer",
                    printers.iter().map(|p| p.label().into()).collect(),
                    Some(choice.printer_index),
                    Edit::Printer
                )
            ),
            row![
                field("shell-print-copies", copies.into_element()),
                field(
                    "shell-print-pages",
                    combo(
                        window,
                        "shell-print-pages",
                        mode_labels,
                        mode_index,
                        move |index| Edit::Pages(page_modes[index])
                    )
                ),
            ]
            .spacing(12),
        ]
        .spacing(12);
        if choice.pages == Pages::Range {
            let range = enz_text_field("", "1-3, 5", &choice.page_ranges, move |value| {
                Message::PrintEdit(window, Edit::Range(value))
            })
            .aria_label(tr("shell-print-page-range"))
            .id("print-range".into());
            let range = if choice.page_ranges.is_empty() || choice.valid_range() {
                range
            } else {
                range.error(tr("shell-print-range-error"))
            };
            fields = fields.push(field("shell-print-page-range", range.into_element()));
        }
        let orientations = Orientation::ALL;
        let orientation = orientations
            .iter()
            .position(|mode| *mode == choice.orientation);
        let mut layout = row![field(
            "shell-print-orientation",
            combo(
                window,
                "shell-print-orientation",
                orientations
                    .iter()
                    .copied()
                    .map(|m| orientation_label(m).into())
                    .collect(),
                orientation,
                |index| Edit::Orientation(orientations[index])
            )
        )]
        .spacing(12);
        if !printer.papers().is_empty() {
            layout = layout.push(field(
                "shell-print-paper",
                combo(
                    window,
                    "shell-print-paper",
                    printer
                        .papers()
                        .iter()
                        .map(|paper| paper.label().into())
                        .collect(),
                    Some(choice.paper),
                    Edit::Paper,
                ),
            ));
        }
        fields = fields.push(layout);
        let duplex = printer.duplex();
        let color = printer.color();
        let mut output = row![].spacing(12);
        if duplex.len() > 1 {
            output = output.push(field(
                "shell-print-duplex",
                combo(
                    window,
                    "shell-print-duplex",
                    duplex
                        .iter()
                        .copied()
                        .map(|m| duplex_label(m).into())
                        .collect(),
                    duplex.iter().position(|mode| Some(*mode) == choice.duplex),
                    move |index| Edit::Duplex(duplex[index]),
                ),
            ));
        }
        if color.len() > 1 {
            output = output.push(field(
                "shell-print-color-mode",
                combo(
                    window,
                    "shell-print-color-mode",
                    color
                        .iter()
                        .map(|color| {
                            tr(if *color {
                                "shell-print-color"
                            } else {
                                "shell-print-monochrome"
                            })
                            .into()
                        })
                        .collect(),
                    color.iter().position(|mode| Some(*mode) == choice.color),
                    move |index| Edit::Color(color[index]),
                ),
            ));
        }
        if duplex.len() > 1 || color.len() > 1 {
            fields = fields.push(output);
        }
        fields.into()
    }
}

fn field<'a>(label: &'static str, control: Element<'a, Message>) -> Element<'a, Message> {
    column![
        enz_text(tr(label)).size(TextSize::Footnote).into_element(),
        control
    ]
    .spacing(4)
    .width(Length::Fill)
    .into()
}

fn combo<'a>(
    window: window::Id,
    label: &'static str,
    options: Vec<String>,
    selected: Option<usize>,
    edit: impl Fn(usize) -> Edit + 'a,
) -> Element<'a, Message> {
    enz_combo_box("", options, None, |_| Message::Armed)
        .on_select_index(move |index| Message::PrintEdit(window, edit(index)))
        .selected_index(selected)
        .aria_label(tr(label))
        .id(label.into())
        .width(Length::Fill)
        .into_element()
}
