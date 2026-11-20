use crate::Message;
use enzui_widgets::motion::SlideDirection;
use iced::{
    Element, Length,
    widget::{container, mouse_area},
};

#[allow(
    clippy::too_many_arguments,
    reason = "The region combines caller-owned presentation and input state."
)]
pub(crate) fn view<'a>(
    content: Element<'a, Message>,
    visible: bool,
    direction: SlideDirection,
    hidden: Message,
    theme: &iced::Theme,
    reveal: enzui_shell::reveal::Reveal,
    input: Option<enzui_shell::surface_input::SurfaceInput>,
    expressive: Option<enzui_theme::ExpressiveMotion>,
) -> Element<'a, Message> {
    view_fitted(
        content, visible, direction, hidden, theme, reveal, input, None, None, expressive,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "The view composes independent state and actions supplied by its caller."
)]
pub(crate) fn view_fitted<'a>(
    content: Element<'a, Message>,
    visible: bool,
    direction: SlideDirection,
    hidden: Message,
    theme: &iced::Theme,
    reveal: enzui_shell::reveal::Reveal,
    input: Option<enzui_shell::surface_input::SurfaceInput>,
    extent: Option<(iced::advanced::widget::Id, f32)>,
    glass: Option<enzui_theme::MaterialStyle>,
    expressive: Option<enzui_theme::ExpressiveMotion>,
) -> Element<'a, Message> {
    let fit = extent.map(|value| vec![value]).or_else(|| {
        matches!(&hidden, Message::ControlsHidden(_)).then(|| {
            let inset = enzui_shell::panel_style::INSET * 2.0 + reveal.join();
            vec![
                (
                    "control-grid-extent".into(),
                    inset + enzui_shell::control_layout::ACTIONS_ROW,
                ),
                ("control-body-extent".into(), inset),
            ]
        })
    });
    let margins = reveal.margins();
    mouse_area(
        container(enzui_shell::presentation::attached_fitted(
            container(mouse_area(content).on_press(Message::RegionInterior))
                .padding(margins)
                .into(),
            visible,
            direction,
            theme,
            Some(hidden),
            reveal,
            fit,
            input,
            glass,
            expressive,
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true),
    )
    .on_press(if reveal.is_island() {
        Message::RegionInterior
    } else {
        Message::DismissRegions
    })
    .into()
}

pub(crate) struct Popup {
    pub visible: bool,
    pub bottom: bool,
    pub hidden: Message,
    pub theme: iced::Theme,
    pub attachment: enzui_shell::reveal::Attachment,
    pub radius: f32,
    pub color: iced::Color,
    pub expressive: Option<enzui_theme::ExpressiveMotion>,
    pub input: Option<enzui_shell::surface_input::SurfaceInput>,
    /// The panel's shadow and the room its window keeps for one.
    pub shadow: enzui_shell::reveal::PanelShadow,
    /// Liquid glass for a floating panel's body, when the compositor paints it.
    pub glass: Option<enzui_theme::MaterialStyle>,
}

pub(crate) fn popup(content: Element<'_, Message>, popup: Popup) -> Element<'_, Message> {
    popup_fitted(content, popup, None)
}

pub(crate) fn popup_fitted<'a>(
    content: Element<'a, Message>,
    popup: Popup,
    extent: Option<(iced::advanced::widget::Id, f32)>,
) -> Element<'a, Message> {
    view_fitted(
        content,
        popup.visible,
        if popup.bottom {
            SlideDirection::Down
        } else {
            SlideDirection::Up
        },
        popup.hidden,
        &popup.theme,
        enzui_shell::reveal::Reveal::popup(
            popup.bottom,
            popup.attachment,
            popup.radius,
            popup.color,
            popup.shadow,
        ),
        popup.input,
        extent,
        popup.glass,
        popup.expressive,
    )
}
