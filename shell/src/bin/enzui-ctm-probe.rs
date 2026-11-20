//! Test probe: prints whether a client (the shell's night light) holds the
//! compositor's colour transform. Exits 2 when it cannot tell.
#![allow(
    clippy::print_stdout,
    reason = "This command writes its result or probe protocol to stdout; diagnostics use stderr."
)]

use enzui_shell::i18n;
fn main() {
    i18n::initialize();
    match enzui_shell::night_light::held() {
        Ok(true) => println!("held"),
        Ok(false) => println!("free"),
        Err(error) => {
            eprintln!("enzui-ctm-probe: {error}");
            std::process::exit(2);
        }
    }
}
