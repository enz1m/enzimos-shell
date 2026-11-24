use crate::bar_layout::{BarLayout, Section};
use enzui_core::drag::{DragOperation, DropRequest};
use enzui_shell::i18n::tr;
use enzui_theme::EnzTheme;
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    drag_drop_host::{DragHostChannel, enz_drag_drop_host},
    icon::enz_icon,
    reorder_list::enz_reorder_list,
    text::enz_text,
};
use iced::{
    Element, Length,
    widget::{column, container, row, space},
};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub enum Action {
    Drop(DropRequest),
    Visible(String, bool),
    Step(String, i32),
    ToSection(String, Section),
    Reset,
}
// The tray of items that are not shown in the bar.
const HIDDEN: &str = "hidden";
// The full-list slot for a drop at `visible` among a section's shown items.
fn shown_slot(layout: &BarLayout, section: Section, visible: usize) -> usize {
    let items = layout.items(section);
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.enabled())
        .nth(visible)
        .map_or(items.len(), |(index, _)| index)
}
impl Action {
    pub fn apply(self, layout: &mut BarLayout) -> Result<(), String> {
        match self {
            Self::Drop(drop) => {
                if drop.operation != DragOperation::Move {
                    return Err(tr("shell-only-bar-moves-are-supported").into());
                }
                let ids: Vec<_> = drop
                    .items
                    .iter()
                    .map(enzui_core::drag::ItemId::as_str)
                    .collect();
                match (drop.source.as_str(), drop.destination.as_str()) {
                    (HIDDEN, HIDDEN) => {}
                    (_, HIDDEN) => {
                        for id in &ids {
                            layout.set_enabled(id, false)?;
                        }
                    }
                    (source, destination) => {
                        let destination = Section::parse(destination)?;
                        let slot = shown_slot(layout, destination, drop.slot.index());
                        for id in &ids {
                            let from = if source == HIDDEN {
                                layout
                                    .section_of(id)
                                    .ok_or_else(|| tr("shell-bar-element-is-no-longer-present"))?
                            } else {
                                Section::parse(source)?
                            };
                            layout.move_to_slot(
                                from,
                                destination,
                                &[id],
                                slot.min(layout.items(destination).len()),
                            )?;
                            layout.set_enabled(id, true)?;
                        }
                    }
                }
            }
            Self::Visible(id, enabled) => {
                layout.set_enabled(&id, enabled)?;
            }
            Self::Step(id, delta) => {
                if delta != -1 && delta != 1 {
                    return Err(tr("shell-invalid-bar-move-direction").into());
                }
                let section = layout
                    .section_of(&id)
                    .ok_or_else(|| tr("shell-bar-element-is-no-longer-present"))?;
                let rows = layout.items(section);
                let index = rows
                    .iter()
                    .position(|i| i.id() == id)
                    .expect("the normalized layout contains each selected row");
                let destination = index as i32 + delta;
                if destination < 0 || destination >= rows.len() as i32 {
                    return Ok(());
                }
                let slot = if delta < 0 { index - 1 } else { index + 2 };
                layout.move_to_slot(section, section, &[&id], slot)?;
            }
            Self::ToSection(id, section) => {
                layout.move_before(&[&id], section, None)?;
            }
            Self::Reset => *layout = BarLayout::default().with_notifications(),
        }
        Ok(())
    }
}
pub fn label(id: &str) -> &str {
    match id {
        "builtin.applications" => tr("shell-applications"),
        "builtin.desktops" => tr("shell-desktops"),
        "builtin.clock" => tr("shell-time"),
        "builtin.controls" => tr("shell-control-center-2"),
        "builtin.notifications" => tr("shell-notifications"),
        "plugin:clipboard/history" => tr("shell-clipboard"),
        _ => id,
    }
}
// What the bar draws for `id`, at the bar's own sizes.
fn element<'a, Message: 'a>(
    id: &str,
    palette: &enzui_theme::Palette,
    text: f32,
    plugin_names: &BTreeMap<String, String>,
) -> Element<'a, Message> {
    let ink = palette.text;
    let icon = |name: &'static str| enz_icon(name).size(14.0).tint(ink).into_element();
    let words = |value: String| {
        enz_text(value)
            .size_px(text)
            .weight_rung(enzui_theme::WeightRung::Semibold)
            .into_element()
    };
    match id {
        "builtin.applications" => row![icon("layout-grid"), words(tr("shell-applications").into())]
            .spacing(6)
            .align_y(iced::Alignment::Center)
            .into(),
        "builtin.desktops" => {
            let mark = |width: f32| -> Element<'a, Message> {
                container(space().width(width).height(4.0))
                    .style(move |_: &iced::Theme| container::Style {
                        background: Some(ink.into()),
                        border: iced::Border {
                            radius: 2.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .into()
            };
            row![mark(16.0), mark(4.0)]
                .spacing(6)
                .align_y(iced::Alignment::Center)
                .into()
        }
        "builtin.clock" => words({
            let now = chrono::Local::now();
            format!(
                "{}   {}",
                crate::i18n::clock_date(now.date_naive(), true),
                crate::i18n::time(&now, true)
            )
        }),
        "builtin.notifications" => icon("bell"),
        "builtin.controls" => row![
            icon("volume-2"),
            icon("ethernet-port"),
            icon("sliders-horizontal")
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center)
        .into(),
        "plugin:clipboard/history" => row![
            icon("clipboard"),
            words(enzui_shell::i18n::tr("shell-clipboard").into())
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center)
        .into(),
        // Another plugin's element goes by its plugin's name, kept to a
        // chip's width.
        other => {
            let name = other
                .strip_prefix("plugin:")
                .and_then(|rest| rest.split_once('/'))
                .and_then(|(plugin, _)| plugin_names.get(plugin))
                .map_or_else(|| label(other).to_owned(), Clone::clone);
            row![
                icon("puzzle"),
                enz_text(name)
                    .size_px(text)
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .wrapping(iced::widget::text::Wrapping::None)
                    .ellipsis(iced::widget::text::Ellipsis::End)
                    .width(Length::Shrink.max(140.0))
                    .into_element()
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center)
            .into()
        }
    }
}
// A chip's height: smaller than the bar's own islands, so three zones and
// the tray fit a Settings window that is not maximized.
const CHIP: f32 = 28.0;
// A draggable bar element.
fn chip<'a, Message: 'a>(
    id: &str,
    palette: &enzui_theme::Palette,
    text: f32,
    shown: bool,
    plugin_names: &BTreeMap<String, String>,
) -> Element<'a, Message> {
    let fill = if shown {
        palette.surface_alt
    } else {
        palette.surface
    };
    let edge = palette.border;
    container(element(id, palette, text, plugin_names))
        .padding([0, 10])
        .height(CHIP)
        .align_y(iced::Alignment::Center)
        .style(move |_| container::Style {
            background: Some(fill.into()),
            border: iced::Border {
                radius: (CHIP / 2.0).into(),
                width: 1.0,
                color: edge,
            },
            ..Default::default()
        })
        .into()
}
/// `plugin_names` names installed plugins by id, for their elements' chips.
pub fn view<'a, Message: Clone + 'a>(
    layout: &'a BarLayout,
    plugin_names: &BTreeMap<String, String>,
    theme: EnzTheme,
    enabled: bool,
    reset_enabled: bool,
    channel: &DragHostChannel,
    action: fn(Action) -> Message,
) -> Element<'a, Message> {
    let palette = &theme.palette;
    let (muted, edge, surface) = (palette.text_muted, palette.border, palette.surface);
    let text = 12.0 * theme.typography.text_scale;
    let list = |collection: &'static str,
                items: Vec<(String, Element<'a, Message>)>|
     -> Element<'a, Message> {
        let empty = items.is_empty();
        let list: Element<'a, Message> =
            enz_reorder_list(items, move |drop| action(Action::Drop(drop)))
                .collection_id(collection)
                .drag_host(channel)
                .horizontal()
                .row_radius(CHIP / 2.0)
                .spacing(6.0)
                .width(if empty {
                    Length::Fixed(72.0)
                } else {
                    Length::Shrink
                })
                .height(Length::Fixed(CHIP))
                .into_element();
        if empty {
            iced::widget::stack![
                list,
                container(enz_icon("plus").size(14.0).tint(muted).into_element())
                    .center(Length::Fill)
                    .style(move |_| container::Style {
                        border: iced::Border {
                            radius: (CHIP / 2.0).into(),
                            width: 1.0,
                            color: edge
                        },
                        ..Default::default()
                    })
            ]
            .width(72)
            .height(CHIP)
            .into()
        } else {
            list
        }
    };
    let shown = |section: Section| -> Vec<(String, Element<'a, Message>)> {
        layout
            .items(section)
            .iter()
            .filter(|item| item.enabled())
            .map(|item| {
                (
                    item.id().to_owned(),
                    chip(item.id(), palette, text, true, plugin_names),
                )
            })
            .collect()
    };
    let hidden: Vec<(String, Element<'a, Message>)> = Section::ALL
        .into_iter()
        .flat_map(|section| layout.items(section).iter())
        .filter(|item| !item.enabled())
        .map(|item| {
            (
                item.id().to_owned(),
                chip(item.id(), palette, text, false, plugin_names),
            )
        })
        .collect();
    // One labelled row for each zone of the bar, divided by hairlines, with
    // its items resting where the zone sits: left, centred, right. A mock of
    // the whole bar on one line did not fit a window narrower than the
    // screen and gave no sign of where one zone ended.
    let zone = |name: &'static str,
                icon: &'static str,
                align: iced::alignment::Horizontal,
                items: Element<'a, Message>|
     -> Element<'a, Message> {
        row![
            row![
                enz_icon(icon).size(14.0).tint(muted).into_element(),
                enz_text(name).size_px(text).color(muted).into_element()
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .width(92),
            container(items).width(Length::Fill).align_x(align),
        ]
        .spacing(8)
        .padding([8, 12])
        .align_y(iced::Alignment::Center)
        .into()
    };
    let divider = || -> Element<'a, Message> {
        container(space().width(Length::Fill).height(1.0))
            .style(move |_| container::Style {
                background: Some(edge.into()),
                ..Default::default()
            })
            .into()
    };
    use iced::alignment::Horizontal;
    let bar = container(column![
        zone(
            tr("shell-left"),
            "panel-left",
            Horizontal::Left,
            list(Section::Start.key(), shown(Section::Start)),
        ),
        divider(),
        zone(
            tr("shell-center"),
            "align-horizontal-space-around",
            Horizontal::Center,
            list(Section::Middle.key(), shown(Section::Middle)),
        ),
        divider(),
        zone(
            tr("shell-right"),
            "panel-right",
            Horizontal::Right,
            list(Section::End.key(), shown(Section::End)),
        ),
    ])
    .width(Length::Fill)
    .style(move |_| container::Style {
        background: Some(surface.into()),
        border: iced::Border {
            radius: 12.0.into(),
            width: 1.0,
            color: edge,
        },
        ..Default::default()
    });
    let tray = zone(
        tr("shell-bar-hidden"),
        "eye-off",
        Horizontal::Left,
        list(HIDDEN, hidden),
    );
    let content: Element<'a, Message> = column![bar, tray].spacing(4).into();
    let content: Element<'a, Message> = if enabled {
        enz_drag_drop_host(channel, content)
            .on_drop(move |drop| action(Action::Drop(drop)))
            .into()
    } else {
        content
    };
    column![
        content,
        row![
            space::horizontal(),
            enz_button(tr("shell-reset-bar-layout"), action(Action::Reset))
                .id("bar-layout-reset".into())
                .kind(ButtonKind::Quiet)
                .icon("rotate-ccw")
                .enabled(enabled && reset_enabled)
                .into_element()
        ]
    ]
    .spacing(12)
    .into()
}
