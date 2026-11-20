//! Optional desktop message and frame diagnostics.
use std::{path::PathBuf, time::Instant};

// A runtime trace-shell marker enables ten-second message and no-redraw counters.
type MessageCounts = std::collections::BTreeMap<&'static str, (u32, u32)>;
pub(super) struct MessageTrace {
    counts: std::sync::Mutex<(Instant, MessageCounts)>,
}
impl MessageTrace {
    pub(super) fn record(&self, kind: &'static str, quiet: bool) {
        let Ok(mut state) = self.counts.lock() else {
            return;
        };
        let entry = state.1.entry(kind).or_default();
        entry.0 += 1;
        entry.1 += u32::from(quiet);
        if state.0.elapsed() >= std::time::Duration::from_secs(10) {
            let report: Vec<String> = state
                .1
                .iter()
                .map(|(kind, (all, quiet))| format!("{kind} {all}/{quiet}q"))
                .collect();
            eprintln!("enzui-shell trace 10s: {}", report.join(" | "));
            *state = (Instant::now(), Default::default());
        }
    }
}
pub(super) fn message_trace() -> Option<&'static MessageTrace> {
    static TRACE: std::sync::OnceLock<Option<MessageTrace>> = std::sync::OnceLock::new();
    TRACE
        .get_or_init(|| {
            let run = std::env::var_os("ENZUI_HYPR_RUNROOT")?;
            PathBuf::from(run)
                .join("trace-shell")
                .exists()
                .then(|| MessageTrace {
                    counts: std::sync::Mutex::new((Instant::now(), Default::default())),
                })
        })
        .as_ref()
}

#[cfg(feature = "frame-report")]
pub(super) fn frame_report() {
    let Some(path) = std::env::var_os("ENZUI_FRAME_REPORT")
        .or_else(|| {
            Some(
                PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?)
                    .join("enzui-frame-report.ndjson")
                    .into(),
            )
        })
        .map(PathBuf::from)
    else {
        return;
    };
    iced::performance::enable(iced::performance::Config::default());
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let staged = path.with_extension("partial");
            if std::fs::write(&staged, iced::performance::to_ndjson()).is_ok() {
                let _ = std::fs::rename(&staged, &path);
            }
        }
    });
}
