//! Opens the first-login Welcome.
#[path = "../welcome_app.rs"]
mod welcome_app;

fn main() -> std::process::ExitCode {
    enzui_shell::i18n::initialize();
    welcome_app::main()
}
