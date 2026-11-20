//! Starts the greetd greeter on the current Wayland display.
use enzui_shell::i18n;
fn main() -> std::process::ExitCode {
    i18n::initialize();
    match enzui_lock::run(enzui_lock::Mode::Greeter) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("enzui-greeter: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
