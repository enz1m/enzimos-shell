//! Opens desktop Settings with session-aware access to system controls.
use enzui_shell::i18n;
#[path = "../settings_app.rs"]
mod settings_app;

fn main() -> std::process::ExitCode {
    i18n::initialize();
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|arg| arg == enzui_shell::controller_vibration::WORKER_ARGUMENT)
    {
        std::process::exit(enzui_shell::controller_vibration::worker(&arguments[1..]));
    }
    settings_app::main()
}
