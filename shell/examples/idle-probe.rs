//! Reports compositor idle transitions through the shell subscription.
#![allow(
    clippy::print_stdout,
    reason = "This command writes its result or probe protocol to stdout; diagnostics use stderr."
)]
use enzui_shell::idle::{Config, Event};
use std::{io::BufRead, os::unix::net::UnixStream, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("ENZUI_IDLE_QA").as_deref() != Ok("owned-preview")
        || std::env::var("ENZUI_DESKTOP_SESSION").as_deref() != Ok("preview")
    {
        return Err("Idle probe requires an explicitly owned preview".into());
    }
    drop(enzui_shell::clipboard::session::Target::from_environment()?.connect()?);
    let (stop, keepalive) = UnixStream::pair()?;
    let (events, mut receiver) = tokio::sync::mpsc::channel(4);
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            if matches!(line.as_deref(), Ok("stop")) || line.is_err() {
                break;
            }
        }
        drop(keepalive);
    });
    let thread = std::thread::spawn(move || {
        enzui_shell::idle::run(
            Config {
                blank: Some(Duration::from_secs(2)),
                lock: None,
                suspend: None,
            },
            stop,
            events,
        )
    });
    while let Some(event) = receiver.blocking_recv() {
        match event {
            Event::Unavailable(error) => eprintln!("Unavailable: {error}"),
            Event::Awake => println!("Awake"),
            _ => return Err("Probe unexpectedly requested lock or suspend".into()),
        }
    }
    let result = thread.join().map_err(|_| "Idle worker panicked")?;
    println!("Stopped");
    result?;
    Ok(())
}
