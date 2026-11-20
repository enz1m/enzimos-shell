use super::SecretText;
use enzui_shell::i18n::tr;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn helper() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("enzui-password-hash")))
        .filter(|p| p.is_file())
        .or_else(|| {
            [
                "/usr/lib/enzui/bin/enzui-password-hash",
                env!("ENZUI_PASSWORD_HASHER"),
            ]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.is_absolute() && p.is_file())
        })
}

pub(super) async fn hash(secret: SecretText) -> Result<SecretText, String> {
    let executable =
        helper().ok_or_else(|| tr("shell-password-setup-is-unavailable-on-this-installation"))?;
    let mut child = tokio::process::Command::new(executable)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| tr("shell-password-setup-could-not-start-try-again"))?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| tr("shell-cannot-send-the-password-securely"))?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| tr("shell-cannot-read-the-password-helper-response"))?;
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        input
            .write_all(secret.expose_secret().as_bytes())
            .await
            .map_err(|_| tr("shell-cannot-send-the-password-securely"))?;
        drop(input);
        let mut bytes = Vec::new();
        output
            .take(1025)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| tr("shell-cannot-read-the-password-helper-response"))?;
        if bytes.len() > 1024 {
            return Err(tr("shell-password-helper-response-exceeded-its-limit"));
        }
        let status = child
            .wait()
            .await
            .map_err(|_| tr("shell-cannot-wait-for-the-password-helper"))?;
        if !status.success() {
            return Err(tr("shell-the-system-could-not-prepare-this-password"));
        }
        let mut encoded =
            String::from_utf8(bytes).map_err(|_| tr("shell-invalid-password-helper-response"))?;
        if encoded.ends_with('\n') {
            encoded.pop();
        }
        if !encoded.starts_with('$')
            || encoded.len() < 20
            || encoded.chars().any(char::is_whitespace)
        {
            return Err(tr("shell-invalid-password-helper-response"));
        }
        Ok(SecretText::new(encoded))
    })
    .await;
    match result {
        Ok(Ok(encoded)) => Ok(encoded),
        failure => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            Err(match failure {
                Ok(Err(error)) => error,
                _ => tr("shell-password-helper-timed-out"),
            }
            .into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn password_hashes_are_salted_and_secret_formatting_is_redacted() {
        let secret = SecretText::new("test-only-initial-password".into());
        assert!(!format!("{secret:?}").contains("test-only"));
        let first = hash(secret.clone()).await.unwrap();
        let second = hash(secret).await.unwrap();
        assert_ne!(first.expose_secret(), second.expose_secret());
        assert!(
            first.expose_secret().starts_with("$y$") || first.expose_secret().starts_with("$6$")
        );
        assert!(!format!("{first:?}").contains(first.expose_secret()));
    }
}
