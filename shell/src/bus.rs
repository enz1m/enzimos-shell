//! Shared D-Bus connections for the shell's periodic reads.
//!
//! A connection is a socket, an authentication exchange and a name on the
//! bus; reads that run every few seconds reuse one instead of opening and
//! closing their own each time.

use std::time::Duration;
use tokio::sync::OnceCell;

/// How long a read of a system service may take before it counts as
/// unanswered: a stuck service must not hold the control center's refresh.
pub const READ: Duration = Duration::from_secs(3);

/// `work` if it finishes within [`READ`].
pub async fn bounded<T>(work: impl std::future::Future<Output = T>) -> Option<T> {
    tokio::time::timeout(READ, work).await.ok()
}

/// The system bus, connected on first use; `None` when it cannot be reached.
pub async fn system() -> Option<zbus::Connection> {
    static SYSTEM: OnceCell<Option<zbus::Connection>> = OnceCell::const_new();
    SYSTEM
        .get_or_init(|| async { zbus::Connection::system().await.ok() })
        .await
        .clone()
}

/// The session bus, connected on first use; `None` when it cannot be reached.
pub async fn session() -> Option<zbus::Connection> {
    static SESSION: OnceCell<Option<zbus::Connection>> = OnceCell::const_new();
    SESSION
        .get_or_init(|| async { zbus::Connection::session().await.ok() })
        .await
        .clone()
}
