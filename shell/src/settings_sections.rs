use enzui_widgets::card::{CardRhythm, CardTone, EnzCard, enz_card};
use iced::{Element, Length, Padding};

pub(super) struct SectionCard<'a, Message> {
    card: EnzCard<'a, Message>,
    anchor: iced::advanced::widget::Id,
}

impl<'a, Message: Clone + 'a> SectionCard<'a, Message> {
    pub(super) fn title(mut self, title: &str) -> Self {
        self.card = self.card.title(title);
        self
    }

    pub(super) fn description(mut self, description: &str) -> Self {
        self.card = self.card.description(description);
        self
    }

    pub(super) fn into_element(self) -> Element<'a, Message> {
        iced::widget::container(self.card.into_element())
            .id(self.anchor)
            .width(Length::Fill)
            .into()
    }
}

impl<'a, Message: Clone + 'a> From<SectionCard<'a, Message>> for Element<'a, Message> {
    fn from(section: SectionCard<'a, Message>) -> Self {
        section.into_element()
    }
}

// These compact heading sizes are pins, and an EnzCard size pin wins over its
// own text-scale following, so the pinned sizes carry the desktop scale here.
fn section_rhythm(scale: f32) -> CardRhythm {
    CardRhythm {
        gap: Some(12.0),
        title_size: Some(17.0 * scale),
        description_size: Some(12.0 * scale),
        ..Default::default()
    }
}

pub(super) fn section_with_scale<'a, Message: Clone + 'a>(
    scale: f32,
    content: impl Into<Element<'a, Message>>,
) -> EnzCard<'a, Message> {
    enz_card(enz_card(content).padding(16).into_element())
        .tone(CardTone::Flat)
        .padding(Padding::ZERO)
        .rhythm(section_rhythm(scale))
}

pub(super) fn section_for_scaled<'a, Message: Clone + 'a>(
    scale: f32,
    route: &str,
    content: impl Into<Element<'a, Message>>,
) -> SectionCard<'a, Message> {
    SectionCard {
        card: section_with_scale(scale, content),
        anchor: format!("section-{route}").into(),
    }
}

// A titled section whose content brings its own cards, so a long list can be
// split into sub-headed groups under one heading.
pub(super) fn grouped_section_for_scaled<'a, Message: Clone + 'a>(
    scale: f32,
    route: &str,
    content: impl Into<Element<'a, Message>>,
) -> SectionCard<'a, Message> {
    SectionCard {
        card: enz_card(content)
            .tone(CardTone::Flat)
            .padding(Padding::ZERO)
            .rhythm(section_rhythm(scale)),
        anchor: format!("section-{route}").into(),
    }
}

// The card one group of a grouped section sits in, matching a section's own.
pub(super) fn group_card<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    enz_card(content).padding(16).into_element()
}
