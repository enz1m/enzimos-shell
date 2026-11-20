use super::protocol::{Element as BarElement, Node, NodeKind, Panel, Tone};
use crate::settings::Metrics;
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_theme::EnzTheme;
use enzui_widgets::{
    clickable::enz_clickable,
    divider::enz_divider,
    icon::enz_icon,
    image::{EnzImageFit, enz_image_handle},
    keyed_row::{keyed_column, keyed_row},
    list_row_surface::{TrailingVisibility, enz_list_row_surface},
    scroll_view::enz_scroll_view,
    text::enz_text,
    toggle::enz_toggle,
    toolbar_icon::enz_toolbar_icon,
};
use iced::{
    Element, Length,
    widget::{
        column, container,
        image::Handle,
        responsive, row,
        text::{Ellipsis, Wrapping},
    },
};
use std::rc::Rc;

#[derive(Clone)]
pub struct Interaction {
    pub id: String,
    pub value: Option<bool>,
}
#[derive(Clone)]
pub struct Appearance {
    pub theme: EnzTheme,
    pub metrics: Metrics,
    pub scroll: enzui_widgets::scroll_view::ScrollTranslation,
}
pub struct ChromeActions<Message> {
    pub close: Message,
    pub inspect: Message,
    pub scroll: Message,
}
type ImageResolver<'a> = Rc<dyn Fn(&str) -> Option<Handle> + 'a>;
#[derive(Clone)]
struct Context<'a, Message> {
    prefix: String,
    theme: EnzTheme,
    metrics: Metrics,
    action: Rc<dyn Fn(Interaction) -> Message + 'a>,
    image: ImageResolver<'a>,
    /// In a footer a labelled switch keeps its label beside it instead of
    /// spreading across the panel.
    inline: bool,
}
pub fn bar<'a, Message: Clone + 'a>(
    view: &'a BarElement,
    plugin_name: &str,
    widget_id: String,
    metrics: Metrics,
    open: Message,
) -> Element<'a, Message> {
    let mut content = row![
        enz_icon(&view.icon)
            .size(f32::from(metrics.icon))
            .into_element()
    ]
    .spacing(7)
    .align_y(iced::Alignment::Center);
    if !view.label.is_empty() {
        content = content.push(
            enz_text(&view.label)
                .size_px(f32::from(metrics.text))
                .wrapping(Wrapping::None)
                .ellipsis(Ellipsis::End)
                .width(Length::Shrink.max(180.0))
                .into_element(),
        );
    }
    crate::bar_item::trigger(enz_clickable(content, open), metrics)
        .aria_label(
            tr!("shell-plugin-2", "arg0" => plugin_name.to_owned(), "arg1" => view.tooltip.clone()),
        )
        .id(widget_id.into())
        .into_element()
}
pub fn panel<'a, Message: Clone + 'a>(
    view: &'a Panel,
    plugin_name: &str,
    prefix: String,
    appearance: Appearance,
    action: impl Fn(Interaction) -> Message + 'a,
    image: impl Fn(&str) -> Option<Handle> + 'a,
    chrome: ChromeActions<Message>,
) -> Element<'a, Message> {
    let ChromeActions {
        close,
        inspect,
        scroll: scrolled,
    } = chrome;
    let Appearance {
        theme,
        metrics,
        scroll,
    } = appearance;
    let p = theme.palette.clone();
    let context = Context {
        prefix: prefix.clone(),
        theme,
        metrics,
        action: Rc::new(action),
        image: Rc::new(image),
        inline: false,
    };
    if let Some(footer) = &view.footer {
        let body: Element<'_, Message> = if view.entry_list {
            let headers = view
                .nodes
                .iter()
                .filter(|n| matches!(n.kind, NodeKind::Text { .. }))
                .map(|n| (key(n), node(n, context.clone())))
                .collect();
            let rows = &view.nodes;
            // With no entries, the plugin's text is the empty state, centred
            // in the panel.
            if !rows
                .iter()
                .any(|n| matches!(n.kind, NodeKind::Entry { .. }))
            {
                container(keyed_column(headers).spacing(6.0))
                    .center(Length::Fill)
                    .into()
            } else {
                let context = context.clone();
                column![
                    keyed_column(headers).spacing(6.0),
                    responsive(move |size| {
                        virtual_entries(
                            rows,
                            context.clone(),
                            scroll.clone(),
                            size.height,
                            scrolled.clone(),
                        )
                    })
                    .height(Length::Fill),
                ]
                .spacing(6.0)
                .height(Length::Fill)
                .into()
            }
        } else {
            enz_scroll_view(
                keyed_column(children(&view.nodes, context.clone()))
                    .spacing(2.0)
                    .width(Length::Fill),
            )
            .backdrop(p.surface)
            .height(Length::Fill)
            .into_element()
        };
        let footer = row![
            enz_toolbar_icon("settings", inspect)
                .aria_label(enzui_shell::tr!("shell-a11y-plugin-open-plugin-settings", "arg0" => enzui_shell::i18n::argument(&(plugin_name))))
                .tooltip(enzui_shell::tr!("shell-a11y-plugin-settings", "arg0" => enzui_shell::i18n::argument(&(plugin_name))))
                .into_element(),
            iced::widget::space().width(Length::Fill),
            keyed_row(children(
                footer,
                Context {
                    inline: true,
                    ..context
                },
            ))
            // Room between footer actions, so an action (clear) never reads
            // as the label of the switch beside it.
            .spacing(16.0)
                .align_y(iced::Alignment::Center)
        ]
        .align_y(iced::Alignment::Center)
        .spacing(6);
        let title = enz_text(&view.title)
            .size_px(crate::panel_style::title_px(f32::from(metrics.text)))
            .weight_rung(enzui_theme::WeightRung::Semibold)
            .into_element();
        // The same inset as the shell's other popovers.
        return container(column![title, body, footer].spacing(crate::panel_style::GAP))
            .padding(crate::panel_style::INSET)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }
    let header = row![
        container(
            enz_text(&view.title)
                .size_px(crate::panel_style::title_px(f32::from(metrics.text)))
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element()
        )
        .width(Length::Fill),
        enz_clickable(
            enz_icon("x").size(f32::from(metrics.icon)).into_element(),
            close
        )
        .id(format!("{prefix}/close").into())
        .aria_label(tr("shell-close-plugin-panel"))
        .padding(6)
        .min_hit(28.0)
        .radius(7.0)
        .border_width(0.0)
        .into_element()
    ]
    .align_y(iced::Alignment::Center)
    .spacing(8);
    let body = keyed_column(children(&view.nodes, context))
        .spacing(8.0)
        .width(Length::Fill);
    let content = column![
        header,
        enz_text(format!("Plugin: {plugin_name}"))
            .size_px(f32::from(metrics.text))
            .muted_on(p.surface)
            .into_element(),
        body
    ]
    .spacing(12)
    .width(Length::Fill);
    enz_scroll_view(
        container(content)
            .padding(crate::panel_style::INSET)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .backdrop(p.surface)
    .into_element()
}
fn icon_toggle<'a, Message: Clone + 'a>(
    value: bool,
    label: String,
    icon: &str,
    id: String,
    change: impl Fn(bool) -> Message + 'a,
) -> Element<'a, Message> {
    use enzui_widgets::{switch::SwitchGeometry, toggle::enz_toggle};
    let control = enz_toggle(value, "", change)
        .geometry(SwitchGeometry {
            track_width: 44.0,
            track_height: 24.0,
            thumb: 16.0,
            inset: 4.0,
        })
        .id(id.into())
        .aria_label(label.clone())
        .thumb_icon(icon)
        .into_element();
    enzui_widgets::tooltip::enz_tooltip(
        control,
        format!("{label} · {}", if value { "On" } else { "Off" }),
        iced::widget::tooltip::Position::Top,
    )
}
/// An entry tile's height and the gap under it.
const ENTRY_ROW: f32 = 36.0;
const ENTRY_GAP: f32 = 4.0;
fn virtual_entries<'a, Message: Clone + 'a>(
    rows: &'a [Node],
    context: Context<'a, Message>,
    scroll: enzui_widgets::scroll_view::ScrollTranslation,
    height: f32,
    scrolled: Message,
) -> Element<'a, Message> {
    let id = format!("{}/entries", context.prefix);
    let offset = scroll.get().y;
    enzui_widgets::virtual_list::enz_virtual_list(
        Rc::new(EntryRows(rows)),
        ENTRY_ROW + ENTRY_GAP,
        move |_, row| node(row, context.clone()),
    )
    .viewport_height(height)
    .scroll_offset(offset)
    .on_scroll(move |viewport| {
        let offset = viewport.absolute_offset();
        scroll.set(iced::Vector::new(offset.x, offset.y));
        scrolled.clone()
    })
    .overscan(2)
    .id(id.into())
    .height(Length::Fill)
    .into_element()
}

struct EntryRows<'a>(&'a [Node]);
impl enzui_core::virtual_sequence::ItemProvider<Node> for EntryRows<'_> {
    fn len(&self) -> u64 {
        self.0
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Entry { .. }))
            .count() as u64
    }
    fn item(&self, index: u64) -> Option<Node> {
        self.0
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Entry { .. }))
            .nth(usize::try_from(index).ok()?)
            .cloned()
    }
}
impl enzui_core::virtual_sequence::KeyedProvider<Node> for EntryRows<'_> {
    fn key(&self, index: u64) -> Option<enzui_core::virtual_sequence::SequenceKey> {
        let row = self
            .0
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Entry { .. }))
            .nth(usize::try_from(index).ok()?)?;
        Some(enzui_core::virtual_sequence::SequenceKey::new(key(row)))
    }
}

fn key(node: &Node) -> String {
    let suffix = match &node.kind {
        NodeKind::Action { id, .. } => format!("action:{id}"),
        NodeKind::Toggle { id, .. } | NodeKind::IconToggle { id, .. } => format!("toggle:{id}"),
        NodeKind::IconAction { id, .. } => format!("icon:{id}"),
        NodeKind::Entry {
            copy,
            delete,
            pin,
            pinned,
            ..
        } => format!("entry:{copy}:{delete}:{pin}:{pinned}"),
        NodeKind::Text { .. } => "text".into(),
        NodeKind::Image { .. } => "image".into(),
        NodeKind::Row(_) => "row".into(),
        NodeKind::Column(_) => "column".into(),
        NodeKind::Separator => "separator".into(),
    };
    format!("{}:{}:{suffix}", node.key.len(), node.key)
}
fn children<'a, Message: Clone + 'a>(
    nodes: &[Node],
    context: Context<'a, Message>,
) -> Vec<(String, Element<'a, Message>)> {
    nodes
        .iter()
        .map(|n| (key(n), node(n, context.clone())))
        .collect()
}
fn node<'a, Message: Clone + 'a>(
    node: &Node,
    context: Context<'a, Message>,
) -> Element<'a, Message> {
    let font = f32::from(context.metrics.text);
    let p = context.theme.palette.clone();
    let widget_id = format!("{}/{}", context.prefix, key(node));
    match &node.kind {
        NodeKind::Text { value, tone } => {
            if *tone == Tone::Danger {
                return enzui_widgets::banner::enz_banner(value.clone())
                    .tone(enzui_widgets::banner::BannerTone::Danger)
                    .into_element();
            }
            let mut text = enz_text(value)
                .size_px(if *tone == Tone::Title {
                    font + 4.0
                } else {
                    font
                })
                .width(Length::Fill);
            if *tone == Tone::Muted {
                text = text.muted_on(p.surface);
            }
            if *tone == Tone::Title {
                text = text.weight_rung(enzui_theme::WeightRung::Semibold);
            }
            text.into_element()
        }
        NodeKind::Action {
            id,
            label,
            detail,
            icon,
            thumbnail,
            enabled,
        } => {
            // In a footer an action is a compact labelled button (a full-width
            // row would push the other footer actions out).
            if context.inline {
                return enzui_widgets::button::enz_button(
                    label.clone(),
                    (context.action)(Interaction {
                        id: id.clone(),
                        value: None,
                    }),
                )
                .kind(enzui_widgets::button::ButtonKind::Secondary)
                .size(enzui_widgets::button::ButtonSize::Sm)
                .enabled(*enabled)
                .id(widget_id.into())
                .into_element();
            }
            let mut content = row![].spacing(10).align_y(iced::Alignment::Center);
            if let Some(handle) = thumbnail.as_deref().and_then(|id| (context.image)(id)) {
                content = content.push(
                    enz_image_handle(handle)
                        .fit(EnzImageFit::Contain)
                        .alt_text(label.clone())
                        .width(Length::Fixed(48.0))
                        .height(Length::Fixed(48.0))
                        .into_element(),
                );
            } else if let Some(icon) = icon {
                content = content.push(
                    enz_icon(icon)
                        .size(f32::from(context.metrics.icon))
                        .into_element(),
                );
            }
            let mut labels = column![
                enz_text(label)
                    .size_px(font)
                    .width(Length::Fill)
                    .into_element()
            ]
            .spacing(3)
            .width(Length::Fill);
            if let Some(detail) = detail {
                labels = labels.push(
                    enz_text(detail)
                        .size_px((font - 1.0).max(10.0))
                        .muted_on(p.surface)
                        .width(Length::Fill)
                        .into_element(),
                );
            }
            content = content.push(labels);
            enz_clickable(
                content,
                (context.action)(Interaction {
                    id: id.clone(),
                    value: None,
                }),
            )
            .id(widget_id.into())
            .aria_label(label.clone())
            .enabled(*enabled)
            .width(Length::Fill)
            .height(Length::Shrink)
            .min_hit(44.0)
            .padding(10)
            .radius(10.0)
            .border_width(0.0)
            .into_element()
        }
        NodeKind::IconAction {
            id,
            label,
            icon,
            enabled,
        } => enz_toolbar_icon(
            icon,
            (context.action)(Interaction {
                id: id.clone(),
                value: None,
            }),
        )
        .id(widget_id.into())
        .aria_label(label.clone())
        .tooltip(label.clone())
        .enabled(*enabled)
        .into_element(),
        NodeKind::IconToggle {
            id,
            label,
            icon,
            value,
        } => {
            let action = std::rc::Rc::clone(&context.action);
            let id = id.clone();
            icon_toggle(*value, label.clone(), icon, widget_id, move |value| {
                action(Interaction {
                    id: id.clone(),
                    value: Some(value),
                })
            })
        }
        NodeKind::Entry {
            label,
            thumbnail,
            copy,
            delete,
            pin,
            pinned,
        } => {
            let command = |id: &String, value| {
                (context.action)(Interaction {
                    id: id.clone(),
                    value,
                })
            };
            let mut content = row![].spacing(6).align_y(iced::Alignment::Center);
            if *pinned {
                content = content.push(enz_icon("pin").size(12.0).into_element());
            }
            if let Some(handle) = thumbnail.as_deref().and_then(|id| (context.image)(id)) {
                content = content.push(
                    enz_image_handle(handle)
                        .alt_text(label.clone())
                        .fit(EnzImageFit::Contain)
                        .width(Length::Fixed(24.0))
                        .height(Length::Fixed(24.0))
                        .into_element(),
                );
            }
            content = content.push(
                enz_text(label)
                    .size_px(font)
                    .wrapping(Wrapping::None)
                    .ellipsis(Ellipsis::End)
                    .width(Length::Fill)
                    .into_element(),
            );
            let tools = row![
                enz_toolbar_icon("trash-2", command(delete, None))
                    .id(format!("{widget_id}/delete").into())
                    .aria_label(enzui_shell::tr!("shell-a11y-delete", "arg0" => enzui_shell::i18n::argument(&(label))))
                    .into_element(),
                enz_toolbar_icon(
                    if *pinned { "pin-off" } else { "pin" },
                    command(pin, Some(!pinned))
                )
                .id(format!("{widget_id}/pin").into())
                .aria_label(format!("{} {label}", if *pinned { enzui_shell::i18n::tr("shell-a11y-unpin") } else { enzui_shell::i18n::tr("shell-a11y-pin") }))
                .into_element(),
                enz_toolbar_icon("copy", command(copy, None))
                    .id(format!("{widget_id}/copy").into())
                    .aria_label(enzui_shell::tr!("shell-a11y-copy", "arg0" => enzui_shell::i18n::argument(&(label))))
                    .into_element()
            ]
            .spacing(2);
            let row =
                enz_list_row_surface(container(content).padding([0, 10]), command(copy, None))
                    .id(widget_id.into())
                    .label(label.clone())
                    .row_height(ENTRY_ROW)
                    .outer_gutter(0.0)
                    .trailing(container(tools).padding(iced::Padding::default().right(4.0)))
                    .trailing_visibility(TrailingVisibility::OnHoverOrFocus)
                    .width(Length::Fill)
                    .into_element();
            // Each entry is its own quiet tile, a small gap apart, so entries
            // read as separate items without lines between them.
            // A faint wash of the ink rather than a surface colour: on light
            // glass the alternate surface matched the panel and the tiles
            // vanished; an ink tint separates on any backdrop.
            let ink = context.theme.palette.text;
            let fill = iced::Color {
                a: if context.theme.palette.is_dark() {
                    0.07
                } else {
                    0.055
                },
                ..ink
            };
            container(container(row).style(move |_| container::Style {
                background: Some(fill.into()),
                border: iced::Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }))
            .padding(iced::Padding::default().bottom(ENTRY_GAP))
            .into()
        }
        NodeKind::Toggle { id, label, value } => {
            let action = std::rc::Rc::clone(&context.action);
            let id = id.clone();
            row![
                enz_text(label)
                    .size_px(font)
                    .width(if context.inline {
                        Length::Shrink
                    } else {
                        Length::Fill
                    })
                    .into_element(),
                enz_toggle(*value, "", move |value| action(Interaction {
                    id: id.clone(),
                    value: Some(value)
                }))
                .id_prefix(widget_id)
                .aria_label(label.clone())
                .into_element()
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center)
            .into()
        }
        NodeKind::Image {
            blob,
            alt,
            width,
            height,
        } => {
            if let Some(handle) = (context.image)(blob) {
                enz_image_handle(handle)
                    .fit(EnzImageFit::Contain)
                    .alt_text(alt.clone())
                    .width(Length::Shrink.max(f32::from(*width)))
                    .height(Length::Shrink.max(f32::from(*height)))
                    .into_element()
            } else {
                enz_text(tr("shell-image-unavailable"))
                    .size_px(font)
                    .muted_on(p.surface)
                    .into_element()
            }
        }
        NodeKind::Column(nodes) => keyed_column(children(nodes, context))
            .spacing(8.0)
            .width(Length::Fill)
            .into(),
        NodeKind::Row(nodes) => {
            let nodes = nodes.clone();
            responsive(move |size| -> Element<'a, Message> {
                let items = children(&nodes, context.clone());
                if size.width < (nodes.len() as f32 * 128.0) {
                    keyed_column(items).spacing(8.0).width(Length::Fill).into()
                } else {
                    keyed_row(items).spacing(8.0).width(Length::Fill).into()
                }
            })
            .height(Length::Shrink)
            .into()
        }
        NodeKind::Separator => enz_divider().into_element(),
    }
}
