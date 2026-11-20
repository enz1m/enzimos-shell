//! Desktop main executable.
use enzui_shell::i18n;
fn main() -> std::process::ExitCode {
    i18n::initialize();
    match enzui_lock::run(enzui_lock::Mode::Lock) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("enzui-lock: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
