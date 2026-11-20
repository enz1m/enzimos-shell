//! Opens the production greeter in a window with inert sign-in actions.
use enzui_shell::i18n;
fn main() -> iced::Result {
    i18n::initialize();
    enzui_lock::run_preview()
}
