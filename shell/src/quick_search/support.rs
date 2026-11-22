use super::model::Message;
use crate::Message as ShellMessage;
use enzui_shell::i18n::tr;
use enzui_shell::search::Category;
use iced::Task;

pub(crate) fn message(value: Message) -> ShellMessage {
    ShellMessage::QuickSearch(value)
}
pub(super) fn perform<T: Send + 'static>(
    future: impl std::future::Future<Output = T> + Send + 'static,
    map: impl Fn(T) -> Message + Send + 'static,
) -> Task<ShellMessage> {
    Task::perform(future, move |value| message(map(value)))
}
pub(super) fn clipboard_connection(epoch: u64) -> Task<ShellMessage> {
    perform(
        async move {
            tokio::task::spawn_blocking(enzui_shell::clipboard::session::Target::from_environment)
                .await
                .map_err(|_| tr("shell-clipboard-scope-check-stopped").to_owned())
                .and_then(|value| value)
        },
        move |value| Message::ClipboardTarget(epoch, value),
    )
}

pub(crate) fn source_chord(
    key: &iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Option<Option<Category>> {
    if !modifiers.control() || modifiers.alt() || modifiers.logo() || modifiers.shift() {
        return None;
    }
    let iced::keyboard::Key::Character(value) = key else {
        return None;
    };
    if value.len() != 1 {
        return None;
    }
    let digit = value.as_bytes()[0];
    (b'0'..=b'8')
        .contains(&digit)
        .then(|| (digit != b'0').then(|| Category::ALL[usize::from(digit - b'1')]))
}

// The expanded card's height apart from its result rows: field, footer and
// padding.
pub(super) const SEARCH_CHROME: f32 = 116.0;
/// The height added by the recommended apps above the full list.
pub(super) const RECOMMENDED_HEIGHT: f32 = 84.0;
/// The gap and height of the Settings and All apps row at the card's foot.
pub(super) const FOOTER_GAP: f32 = 6.0;
pub(super) const FOOTER_HEIGHT: f32 = 32.0;

/// A result row's height with the gap below it.
pub(super) fn row_pitch(compact: bool) -> f32 {
    if compact { 40.0 } else { 48.0 }
}

pub(super) fn result_capacity(
    size: iced::Size,
    chrome: f32,
    compact: bool,
    rows: u8,
    feedback: bool,
) -> usize {
    (((size.height - chrome - if feedback { 40.0 } else { 0.0 }) / row_pitch(compact)).floor()
        as usize)
        .clamp(1, usize::from(rows.max(1)))
}

pub(super) fn focus_if_unfocused<T: Send + 'static>(
    target: iced::advanced::widget::Id,
) -> impl iced::advanced::widget::Operation<T> {
    use iced::advanced::widget::{
        Id, Operation,
        operation::{Focusable, Outcome, focusable},
    };
    struct FocusIfUnfocused {
        target: Id,
        focused: Option<bool>,
    }
    impl<T: Send + 'static> Operation<T> for FocusIfUnfocused {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
            operate(self);
        }
        fn focusable(&mut self, id: Option<&Id>, _: iced::Rectangle, state: &mut dyn Focusable) {
            if id == Some(&self.target) {
                self.focused = Some(self.focused.unwrap_or(false) || state.is_focused());
            }
        }
        fn finish(&self) -> Outcome<T> {
            if self.focused == Some(false) {
                Outcome::Chain(Box::new(focusable::focus(self.target.clone())))
            } else {
                Outcome::None
            }
        }
    }
    FocusIfUnfocused {
        target,
        focused: None,
    }
}

pub(super) fn focus_query_unless_source_focused<T: Send + 'static>()
-> impl iced::advanced::widget::Operation<T> {
    use iced::advanced::widget::{
        Id, Operation,
        operation::{Focusable, Outcome, focusable, then},
    };
    struct SourceFocus(bool);
    impl Operation<bool> for SourceFocus {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<bool>)) {
            operate(self);
        }
        fn focusable(&mut self, id: Option<&Id>, _: iced::Rectangle, state: &mut dyn Focusable) {
            if id == Some(&"quick-search-source".into()) {
                self.0 |= state.is_focused();
            }
        }
        fn finish(&self) -> Outcome<bool> {
            Outcome::Some(self.0)
        }
    }
    then(SourceFocus(false), |focused| {
        focusable::focus(
            if focused {
                "quick-search-source"
            } else {
                "quick-search-input"
            }
            .into(),
        )
    })
}
