use enzui_activity::{Job, Outcome, Progress, Publisher, Status, Unit, Visibility};
use enzui_shell::{i18n::tr, local_ai};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

const UNIT: &str = "enzui-settings-model-downloads";
const STATUS: &str = "enzui-settings-model-download";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) enum Phase {
    Queued,
    Downloading { done: u64, total: u64 },
    Verifying,
    Installed,
    Failed(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Reading {
    pub model: String,
    pub phase: Phase,
    worker: u32,
    start: u64,
}

impl Reading {
    pub(super) fn pending(&self) -> bool {
        matches!(
            self.phase,
            Phase::Queued | Phase::Downloading { .. } | Phase::Verifying
        )
    }

    fn persist(&self) -> Option<()> {
        let file = status_path(&self.model)?;
        let mut staged = tempfile::NamedTempFile::new_in(file.parent()?).ok()?;
        serde_json::to_writer(staged.as_file_mut(), self).ok()?;
        staged.persist(file).ok()?;
        Some(())
    }
}

fn runtime_path() -> Option<PathBuf> {
    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    runtime.is_absolute().then_some(runtime)
}

fn status_path(id: &str) -> Option<PathBuf> {
    (!id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_'))
    .then(|| runtime_path().map(|runtime| runtime.join(format!("{STATUS}-{id}.json"))))
    .flatten()
}

fn started(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

pub(super) fn reading(id: &str) -> Option<Reading> {
    let path = status_path(id)?;
    if path.metadata().ok()?.len() > 4096 {
        return None;
    }
    let mut state: Reading = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    if state.model != id {
        return None;
    }
    if state.pending() && started(state.worker) != Some(state.start) {
        state.phase = Phase::Failed(tr("shell-model-download-interrupted").into());
    }
    Some(state)
}

fn verify_choices(ids: &[String]) -> Result<Vec<local_ai::ModelOffer>, String> {
    if ids.is_empty() || ids.len() > 2 || ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(tr("shell-model-choose-download").into());
    }
    let mut catalog = local_ai::read_model_catalog();
    let choices: Vec<_> = ids
        .iter()
        .filter_map(|id| {
            catalog
                .iter()
                .position(|offer| offer.id == *id && !offer.needs_runtime)
                .map(|index| catalog.swap_remove(index))
        })
        .collect();
    if choices.len() != ids.len() || (choices.len() == 2 && choices[0].speech == choices[1].speech)
    {
        return Err(tr("shell-model-choose-download").into());
    }
    Ok(choices)
}

pub(super) fn queue(ids: &[String], make_default: bool) -> Result<(), String> {
    verify_choices(ids)?;
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut command = Command::new("/usr/bin/systemd-run");
    command
        .args([
            "--user",
            "--quiet",
            "--collect",
            "--unit",
            UNIT,
            "--property=Type=exec",
            "--",
        ])
        .arg(exe)
        .arg("--model-download-worker");
    if make_default {
        command.arg("--default");
    }
    let output = command
        .args(ids)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        Err(error.trim().chars().take(300).collect())
    }
}

fn report(line: &str) -> Option<Phase> {
    if line == "verifying" {
        return Some(Phase::Verifying);
    }
    let (done, total) = line.strip_prefix("progress\t")?.split_once('\t')?;
    let (done, total) = (done.parse().ok()?, total.parse().ok()?);
    (total > 0 && done <= total).then_some(Phase::Downloading { done, total })
}

fn status(label: &str, model: &str) -> Status {
    Status {
        detail: model.to_owned(),
        ..Status::running(label)
    }
}

fn track(phase: &Phase, job: Option<&Job>, model: &str) {
    if let Some(job) = job {
        match *phase {
            Phase::Downloading { done, total } if total > 0 => {
                let _ = job.set_progress(
                    Some(Progress {
                        done,
                        total,
                        unit: Unit::Bytes,
                    }),
                    None,
                );
            }
            Phase::Verifying => {
                let _ = job.update(status(tr("shell-model-verifying"), model));
            }
            _ => {}
        }
    }
}

fn fetch(
    model: &local_ai::ModelOffer,
    make_default: bool,
    state: &mut Reading,
    job: Option<&Job>,
) -> Result<(), String> {
    let errors = tempfile::tempfile().map_err(|error| error.to_string())?;
    let mut command = Command::new(local_ai::MODELS_TOOL);
    command.args(["fetch", &model.id]);
    if make_default {
        command.arg("--default");
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(errors.try_clone().map_err(|error| error.to_string())?)
        .spawn()
        .map_err(|error| error.to_string())?;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(phase) = report(&line) {
                track(&phase, job, &model.title);
                state.phase = phase;
                let _ = state.persist();
            }
        }
    }
    if child.wait().map_err(|error| error.to_string())?.success() {
        return Ok(());
    }
    use std::io::{Read, Seek};
    let mut errors = errors;
    let _ = errors.rewind();
    let mut bytes = Vec::new();
    let _ = errors.take(4096).read_to_end(&mut bytes);
    Err(String::from_utf8_lossy(&bytes).lines().last().map_or_else(
        || tr("shell-model-download-interrupted").to_owned(),
        |line| {
            line.trim_start_matches("enzimos-inference-models: ")
                .chars()
                .take(300)
                .collect()
        },
    ))
}

fn worker(ids: &[String], make_default: bool) -> Result<(), String> {
    let choices = verify_choices(ids)?;
    let unavailable = || tr("shell-model-download-interrupted").to_owned();
    let runtime = runtime_path().ok_or_else(unavailable)?;
    let lock = std::fs::File::create(runtime.join(format!("{STATUS}.lock")))
        .map_err(|error| error.to_string())?;
    lock.try_lock()
        .map_err(|_| tr("shell-model-download-active").to_owned())?;
    let worker = std::process::id();
    let start = started(worker).ok_or_else(unavailable)?;
    let publisher = Publisher::new("dev.enzui.Settings").ok();
    let mut pending: Vec<Reading> = ids
        .iter()
        .map(|id| Reading {
            model: id.clone(),
            phase: Phase::Queued,
            worker,
            start,
        })
        .collect();
    for state in &pending {
        let _ = state.persist();
    }
    let mut failure = None;
    for (model, state) in choices.iter().zip(&mut pending) {
        state.phase = Phase::Downloading { done: 0, total: 0 };
        let _ = state.persist();
        let job = publisher.as_ref().and_then(|publisher| {
            publisher
                .start_with_visibility(
                    status(tr("shell-model-downloading"), &model.title),
                    Visibility::Always,
                )
                .ok()
        });
        let result = fetch(model, make_default, state, job.as_ref());
        state.phase = match result {
            Ok(()) => Phase::Installed,
            Err(error) => {
                failure.get_or_insert_with(|| error.clone());
                Phase::Failed(error)
            }
        };
        let _ = state.persist();
        if let Some(job) = job {
            let _ = job.finish(if state.phase == Phase::Installed {
                Outcome::Succeeded
            } else {
                Outcome::Failed
            });
        }
    }
    failure.map_or(Ok(()), Err)
}

pub(super) fn cli(args: &[OsString]) -> Option<ExitCode> {
    let command = args.first()?.to_str()?;
    let run = match command {
        "--queue-models" => queue,
        "--model-download-worker" => worker,
        _ => return None,
    };
    let rest = args.get(1..)?;
    let make_default =
        command == "--queue-models" || rest.first().is_some_and(|arg| arg == "--default");
    let ids = rest
        .iter()
        .filter(|arg| *arg != "--default")
        .map(|arg| arg.to_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>();
    Some(
        match ids
            .ok_or_else(|| tr("shell-model-choose-download").to_owned())
            .and_then(|ids| run(&ids, make_default))
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                let _ = writeln!(std::io::stderr(), "enzui-settings: {error}");
                ExitCode::FAILURE
            }
        },
    )
}
