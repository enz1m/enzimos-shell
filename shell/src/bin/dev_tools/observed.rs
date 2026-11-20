use enzui_shell::i18n::tr;
use enzui_shell::tr;
pub(crate) fn uptime(started: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> String {
    let Some(started) = started.and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
    else {
        return tr("shell-unknown-uptime").into();
    };
    let seconds = now.signed_duration_since(started).num_seconds();
    if seconds < 0 || started.timestamp() <= 0 {
        return tr("shell-unknown-uptime").into();
    }
    if seconds >= 86400 {
        tr!("shell-d-h-uptime", "arg0" => format!("{:}", seconds/86400), "arg1" => format!("{:}", seconds%86400/3600))
    } else if seconds >= 3600 {
        tr!("shell-h-m-uptime", "arg0" => format!("{:}", seconds/3600), "arg1" => format!("{:}", seconds%3600/60))
    } else {
        tr!("shell-m-s-uptime", "arg0" => format!("{:}", seconds/60), "arg1" => format!("{:}", seconds%60))
    }
}
