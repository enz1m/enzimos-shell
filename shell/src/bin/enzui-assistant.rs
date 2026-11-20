//! Opens the desktop assistant with its configured conversation backend.
use enzui_shell::i18n;
#[path = "../assistant_app.rs"]
mod assistant_app;

fn main() -> iced::Result {
    i18n::initialize();
    assistant_app::run()
}
