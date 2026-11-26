//! `enzui-power-boost`: the D-Bus-activated `dev.enzui.PowerBoost1` helper.

use enzui_power_boost::{cpu, service};
use std::process::ExitCode;
use std::time::Duration;

// systemd keeps this RuntimeDirectory across helper restarts (RuntimeDirectoryPreserve=yes).
const MARKER: &str = "/run/enzui-power-boost/boost";
const IDLE: Duration = Duration::from_secs(60);

fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("enzui-power-boost: {error}");
            return ExitCode::FAILURE;
        }
    };
    let config = service::Config {
        cpufreq: cpu::CPUFREQ.into(),
        marker: MARKER.into(),
        idle: IDLE,
    };
    let result = runtime.block_on(async {
        let connection = zbus::Connection::system().await?;
        service::run(connection, config).await
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("enzui-power-boost: {error}");
            ExitCode::FAILURE
        }
    }
}
