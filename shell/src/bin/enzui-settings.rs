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
    if arguments
        .first()
        .is_some_and(|arg| arg == enzui_shell::system_settings::display::REVERT_GUARD_ARGUMENT)
    {
        std::process::exit(enzui_shell::system_settings::display::revert_guard(
            &arguments[1..],
            restore_displays,
        ));
    }
    settings_app::main()
}

fn restore_displays(layout: Vec<enzui_shell::system_settings::display::Change>) -> bool {
    use enzui_shell::system_settings::{Request, SystemSettings};
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return false;
    };
    runtime.block_on(async {
        SystemSettings::from_environment()
            .apply(&Request::Display(layout))
            .await
            .is_ok()
    })
}
