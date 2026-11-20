//! Serves idle-inhibition requests for an installed local desktop session.
use enzui_shell::i18n::{self, tr};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    i18n::initialize();
    if enzui_shell::desktop_session::mode() != enzui_shell::desktop_session::Mode::Installed
        || enzui_shell::desktop_session::remote()
    {
        return Err(tr("shell-idle-inhibition-requires-an-installed-local-desktop").into());
    }
    let desktop = enzui_shell::clipboard::session::Target::from_environment()?.connect()?;
    desktop.set_nonblocking(true)?;
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        let desktop = tokio::net::UnixStream::from_std(desktop)?;
        let connection = zbus::Connection::session().await?;
        // Keep this exact compositor connection; its end also releases the service.
        tokio::select! {
            result = enzui_shell::idle_inhibit::serve(connection) => result.map_err(Into::into),
            _ = desktop.readable() => Err(tr("shell-the-inhibition-service-s-desktop-ended").into()),
        }
    })
}
