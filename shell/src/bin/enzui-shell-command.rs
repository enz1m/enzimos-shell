//! Sends a command to the selected desktop session.
use enzui_shell::i18n::{self, tr};
use enzui_shell::shortcuts::{Receiver, open_notifications, open_search};

fn main() {
    i18n::initialize();
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let result = if arguments
        .first()
        .is_some_and(|operation| operation == "notifications")
    {
        Receiver::parse(&arguments[1..]).and_then(open_notifications)
    } else if arguments
        .first()
        .is_some_and(|operation| operation == "search")
    {
        Receiver::parse(&arguments[1..]).and_then(open_search)
    } else {
        Err(
            tr("shell-expected-a-scoped-notifications-search-operation-and-receiver-identity")
                .into(),
        )
    };
    if let Err(error) = result {
        eprintln!("Shell shortcut: {error}");
        std::process::exit(1);
    }
}
