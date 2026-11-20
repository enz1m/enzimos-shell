//! Fingerprints through fprintd (`net.reactivated.Fprint`), for Settings.
//!
//! Prints are stored by fprintd (root-only) or on the reader itself; this
//! module only claims the reader for the signed-in user, enrolls and deletes.
//! fprintd asks polkit, which asks for the user's own password before an
//! enrollment (`auth_self_keep`).

use enzui_shell::i18n::tr;
use iced::futures::{SinkExt, StreamExt};
use std::time::Duration;
use zbus::zvariant::OwnedObjectPath;

const SERVICE: &str = "net.reactivated.Fprint";
const MANAGER: &str = "/net/reactivated/Fprint/Manager";
const DEVICE: &str = "net.reactivated.Fprint.Device";
const CALL: Duration = Duration::from_secs(150);
/// An enrollment waits this long for each touch.
const TOUCH: Duration = Duration::from_secs(60);

/// fprintd's finger names, in the order they are offered.
pub const FINGERS: [&str; 10] = [
    "right-index-finger",
    "left-index-finger",
    "right-thumb",
    "left-thumb",
    "right-middle-finger",
    "left-middle-finger",
    "right-ring-finger",
    "left-ring-finger",
    "right-little-finger",
    "left-little-finger",
];

#[must_use]
pub fn finger_label(finger: &str) -> &'static str {
    match finger {
        "right-index-finger" => tr("shell-finger-right-index"),
        "left-index-finger" => tr("shell-finger-left-index"),
        "right-thumb" => tr("shell-finger-right-thumb"),
        "left-thumb" => tr("shell-finger-left-thumb"),
        "right-middle-finger" => tr("shell-finger-right-middle"),
        "left-middle-finger" => tr("shell-finger-left-middle"),
        "right-ring-finger" => tr("shell-finger-right-ring"),
        "left-ring-finger" => tr("shell-finger-left-ring"),
        "right-little-finger" => tr("shell-finger-right-little"),
        "left-little-finger" => tr("shell-finger-left-little"),
        _ => tr("shell-finger-other"),
    }
}

/// The reader and this user's prints.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Overview {
    /// A reader is connected (`None`: fprintd is not installed).
    pub reader: Option<String>,
    pub enrolled: Vec<String>,
    /// The reader is swiped rather than touched.
    pub swipe: bool,
}

/// Progress of an enrollment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// Started: the reader wants this many touches.
    Started(i32),
    /// A touch was accepted.
    Stage,
    /// A touch needs repeating, with the reason.
    Retry(&'static str),
    Done,
    Failed(String),
}

async fn device(system: &zbus::Connection) -> Result<Option<OwnedObjectPath>, String> {
    let manager = zbus::Proxy::new(system, SERVICE, MANAGER, "net.reactivated.Fprint.Manager")
        .await
        .map_err(|_| tr("shell-fingerprint-service-unavailable").to_owned())?;
    match tokio::time::timeout(
        Duration::from_secs(10),
        manager.call::<_, _, OwnedObjectPath>("GetDefaultDevice", &()),
    )
    .await
    {
        Err(_) => Err(tr("shell-fingerprint-service-did-not-answer").into()),
        Ok(Ok(path)) => Ok(Some(path)),
        Ok(Err(zbus::Error::MethodError(name, _, _)))
            if name.as_str().ends_with("NoSuchDevice") =>
        {
            Ok(None)
        }
        Ok(Err(zbus::Error::MethodError(name, _, _)))
            if name.as_str().contains("ServiceUnknown") =>
        {
            Err(String::new())
        }
        Ok(Err(_)) => Err(tr("shell-fingerprint-service-did-not-answer").into()),
    }
}

fn explain(error: &zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(name, _, _) => {
            let name = name.as_str();
            if name.ends_with("PermissionDenied") || name.contains("AccessDenied") {
                tr("shell-fingerprint-not-allowed").into()
            } else if name.ends_with("AlreadyInUse") || name.ends_with("ClaimDevice") {
                tr("shell-fingerprint-reader-busy").into()
            } else if name.ends_with("PrintsNotDeleted") {
                tr("shell-fingerprint-not-deleted").into()
            } else {
                tr("shell-fingerprint-reader-failed").into()
            }
        }
        _ => tr("shell-fingerprint-reader-failed").into(),
    }
}

/// The reader and this user's enrolled fingers.
///
/// # Errors
/// A sentence when fprintd does not answer; an empty sentence when it is not installed.
pub async fn read() -> Result<Overview, String> {
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus").to_owned())?;
    let path = match device(&system).await {
        Err(error) if error.is_empty() => return Ok(Overview::default()),
        Err(error) => return Err(error),
        Ok(None) => {
            return Ok(Overview {
                reader: None,
                enrolled: Vec::new(),
                swipe: false,
            });
        }
        Ok(Some(path)) => path,
    };
    let proxy = zbus::Proxy::new(&system, SERVICE, path, DEVICE)
        .await
        .map_err(|e| explain(&e))?;
    let name: String = proxy.get_property("name").await.unwrap_or_default();
    let scan: String = proxy.get_property("scan-type").await.unwrap_or_default();
    let enrolled: Vec<String> = match tokio::time::timeout(
        Duration::from_secs(10),
        proxy.call("ListEnrolledFingers", &("",)),
    )
    .await
    {
        Ok(Ok(list)) => list,
        Ok(Err(zbus::Error::MethodError(n, _, _))) if n.as_str().ends_with("NoEnrolledPrints") => {
            Vec::new()
        }
        Ok(Err(error)) => return Err(explain(&error)),
        Err(_) => return Err(tr("shell-fingerprint-service-did-not-answer").into()),
    };
    let enrolled = enrolled
        .into_iter()
        .filter(|f| FINGERS.contains(&f.as_str()))
        .collect();
    Ok(Overview {
        reader: Some(if name.is_empty() {
            tr("shell-fingerprint-reader").into()
        } else {
            name
        }),
        enrolled,
        swipe: scan == "swipe",
    })
}

async fn claimed(system: &zbus::Connection) -> Result<zbus::Proxy<'static>, String> {
    let path = device(system)
        .await?
        .ok_or_else(|| tr("shell-fingerprint-no-reader").to_owned())?;
    let proxy = zbus::Proxy::new_owned(system.clone(), SERVICE, path, DEVICE)
        .await
        .map_err(|e| explain(&e))?;
    tokio::time::timeout(
        CALL,
        proxy.call_with_flags::<_, _, ()>(
            "Claim",
            zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
            &("",),
        ),
    )
    .await
    .map_err(|_| tr("shell-fingerprint-service-did-not-answer").to_owned())?
    .map_err(|e| explain(&e))?;
    Ok(proxy)
}

/// Forget one finger, or every finger with `None`.
///
/// # Errors
/// A sentence when fprintd or polkit refused.
pub async fn delete(finger: Option<&str>) -> Result<(), String> {
    if finger.is_some_and(|f| !FINGERS.contains(&f)) {
        return Err(tr("shell-fingerprint-reader-failed").into());
    }
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus").to_owned())?;
    let proxy = claimed(&system).await?;
    let flags = zbus::proxy::MethodFlags::AllowInteractiveAuth.into();
    let result = match finger {
        Some(finger) => {
            tokio::time::timeout(
                CALL,
                proxy.call_with_flags::<_, _, ()>("DeleteEnrolledFinger", flags, &(finger,)),
            )
            .await
        }
        None => {
            tokio::time::timeout(
                CALL,
                proxy.call_with_flags::<_, _, ()>("DeleteEnrolledFingers2", flags, &()),
            )
            .await
        }
    };
    let _ = proxy.call::<_, _, ()>("Release", &()).await;
    result
        .map_err(|_| tr("shell-fingerprint-service-did-not-answer").to_owned())?
        .map(|_| ())
        .map_err(|e| explain(&e))
}

fn retry_reason(result: &str) -> Option<&'static str> {
    Some(match result {
        "enroll-retry-scan" => tr("shell-fingerprint-retry-scan"),
        "enroll-swipe-too-short" => tr("shell-fingerprint-swipe-too-short"),
        "enroll-too-fast" => tr("shell-fingerprint-too-fast"),
        "enroll-finger-not-centered" => tr("shell-fingerprint-not-centered"),
        "enroll-remove-and-retry" => tr("shell-fingerprint-remove-and-retry"),
        _ => return None,
    })
}

fn failure(result: &str) -> String {
    match result {
        "enroll-data-full" => tr("shell-fingerprint-reader-full").into(),
        "enroll-duplicate" => tr("shell-fingerprint-duplicate").into(),
        "enroll-disconnected" => tr("shell-fingerprint-disconnected").into(),
        _ => tr("shell-fingerprint-enroll-failed").into(),
    }
}

/// Enroll `finger`: a stream of progress ending with `Done` or `Failed`.
pub fn enroll(finger: String) -> impl iced::futures::Stream<Item = Progress> {
    iced::stream::channel(
        16,
        async move |mut output: iced::futures::channel::mpsc::Sender<Progress>| {
            let result = enroll_into(&finger, &mut output).await;
            let _ = output
                .send(match result {
                    Ok(()) => Progress::Done,
                    Err(error) => Progress::Failed(error),
                })
                .await;
        },
    )
}

async fn enroll_into(
    finger: &str,
    output: &mut iced::futures::channel::mpsc::Sender<Progress>,
) -> Result<(), String> {
    if !FINGERS.contains(&finger) {
        return Err(tr("shell-fingerprint-reader-failed").into());
    }
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus").to_owned())?;
    let proxy = claimed(&system).await?;
    let outcome = async {
        let stages: i32 = proxy.get_property("num-enroll-stages").await.unwrap_or(-1);
        let mut status = proxy
            .receive_signal("EnrollStatus")
            .await
            .map_err(|e| explain(&e))?;
        proxy
            .call_with_flags::<_, _, ()>(
                "EnrollStart",
                zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
                &(finger,),
            )
            .await
            .map_err(|e| explain(&e))?;
        let _ = output.send(Progress::Started(stages)).await;
        loop {
            let Ok(Some(message)) = tokio::time::timeout(TOUCH, status.next()).await else {
                let _ = proxy.call::<_, _, ()>("EnrollStop", &()).await;
                return Err(tr("shell-fingerprint-timed-out").to_owned());
            };
            let Ok((result, done)) = message.body().deserialize::<(String, bool)>() else {
                continue;
            };
            match result.as_str() {
                "enroll-completed" => return Ok(()),
                "enroll-stage-passed" => {
                    let _ = output.send(Progress::Stage).await;
                }
                other => {
                    if let Some(reason) = retry_reason(other) {
                        let _ = output.send(Progress::Retry(reason)).await;
                    } else if done {
                        return Err(failure(other));
                    }
                }
            }
            if done {
                return Err(failure(&result));
            }
        }
    }
    .await;
    if outcome.is_err() {
        let _ = proxy.call::<_, _, ()>("EnrollStop", &()).await;
    }
    let _ = proxy.call::<_, _, ()>("Release", &()).await;
    outcome
}

/// Sample data for the Settings preview.
#[must_use]
pub fn demo() -> Overview {
    Overview {
        reader: Some("Synaptics Sensors".into()),
        enrolled: vec!["right-index-finger".into()],
        swipe: false,
    }
}
