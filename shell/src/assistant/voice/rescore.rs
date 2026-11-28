use std::{
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{io::AsyncWriteExt as _, process::Command};

#[derive(Debug, Clone)]
pub(crate) struct Transcript {
    pub text: String,
    pub original: Option<String>,
}

impl From<String> for Transcript {
    fn from(text: String) -> Self {
        Self {
            text,
            original: None,
        }
    }
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

async fn proposal(text: &str, previous: &str) -> Option<String> {
    let bin = std::env::var_os("ENZUI_SPEECH_CONTEXT_BIN")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            "/usr/lib/enzimos-inference/streaming-speech/enzimos-speech-context".into()
        });
    let mut child = Command::new(bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;

    let mut input = child.stdin.take()?;
    let previous = previous.split_whitespace().collect::<Vec<_>>().join(" ");
    input
        .write_all(format!("{text}\n{previous}\n").as_bytes())
        .await
        .ok()?;
    drop(input);
    let output = child.wait_with_output().await.ok()?;
    let text = std::str::from_utf8(&output.stdout).ok()?.trim();
    (output.status.success()
        && !text.is_empty()
        && text.len() <= 320
        && !text.contains(['\r', '\n']))
    .then(|| text.to_owned())
}

pub(super) async fn select(
    text: String,
    alternatives: Vec<(String, f64)>,
    previous: &str,
    allowed: impl Fn(&str) -> bool,
) -> Transcript {
    if text.is_empty() || !allowed(&text) || text.len() > 300 {
        return text.into();
    }
    let started = Instant::now();
    let candidate = tokio::time::timeout(Duration::from_secs(2), proposal(&text, previous))
        .await
        .ok()
        .flatten();
    let (result, decision) = match candidate.filter(|candidate| allowed(candidate)) {
        Some(candidate) => validate(text, candidate, &alternatives),
        None => (text.into(), "unavailable"),
    };
    eprintln!(
        "enzui-shell voice context: decision={decision} acoustic_candidates={} elapsed_ms={}",
        alternatives.len(),
        started.elapsed().as_millis()
    );
    result
}

fn validate(
    text: String,
    candidate: String,
    alternatives: &[(String, f64)],
) -> (Transcript, &'static str) {
    let heard = words(&text);
    let revised = words(&candidate);
    let differences = heard
        .iter()
        .zip(&revised)
        .filter(|(before, after)| before != after)
        .count();
    if heard.len() != revised.len() || differences != 1 {
        return (text.into(), "kept");
    }

    let best = alternatives
        .iter()
        .map(|(_, score)| *score)
        .filter(|score| score.is_finite())
        .reduce(f64::max);
    if best.is_some_and(|best| {
        alternatives.iter().any(|(alternative, score)| {
            score.is_finite() && best - score <= 0.20 && words(alternative) == revised
        })
    }) {
        return (candidate.into(), "acoustic");
    }

    (
        Transcript {
            text: candidate,
            original: Some(text),
        },
        "review",
    )
}

#[cfg(test)]
mod tests {
    use super::validate;

    #[test]
    fn contextual_edit_without_acoustic_support_requires_review() {
        let raw = "What zip ties do I have in my downloads folder";
        let suggested = "What zip files do I have in my downloads folder";
        let (result, decision) = validate(raw.into(), suggested.into(), &[(raw.into(), -0.05)]);
        assert_eq!(decision, "review");
        assert_eq!(result.original.as_deref(), Some(raw));
        assert_eq!(result.text, suggested);
        let (result, decision) = validate(
            raw.into(),
            suggested.into(),
            &[(raw.into(), -0.05), (suggested.into(), -0.14)],
        );
        assert_eq!(decision, "acoustic");
        assert!(result.original.is_none());
        assert_eq!(result.text, suggested);
    }
}
