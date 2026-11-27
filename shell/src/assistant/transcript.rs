//! The chat transcript as virtual-list rows: what the service sent, split
//! into rows the panel draws and the list can measure.

use enzui_core::virtual_sequence::{ItemProvider, KeyedProvider, SequenceKey, VariableHeightModel};
use enzui_shell::agent_mock::Item;
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_widgets::{markdown::EnzMarkdownBlock, row_measure::RowMeasurement};
use serde_json::Value;
use std::cell::OnceCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    Pending,
    Sending(bool),
    Allowed,
    Denied,
    Closed,
}

#[derive(Clone, Debug)]
pub(crate) enum Body {
    User {
        text: String,
        steering: bool,
        attachments: Vec<String>,
    },
    Markdown {
        text: Rc<str>,
        blocks: Rc<OnceCell<Vec<EnzMarkdownBlock>>>,
        streaming: bool,
    },
    Image {
        path: String,
        alt: String,
        image: Option<(iced::widget::image::Handle, u32, u32)>,
    },
    Thought {
        text: String,
        millis: u64,
        live: bool,
    },
    Tool {
        call: String,
        tool: String,
        title: String,
        arguments: String,
        output: String,
        status: ToolStatus,
        millis: u64,
    },
    Approval {
        request: String,
        binding: String,
        tool: String,
        title: String,
        arguments: String,
        decision: Decision,
    },
    Note {
        text: String,
        error: bool,
    },
    Meta {
        text: String,
        reply: String,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub key: String,
    pub body: Body,
    pub open: bool,
}

// One piece of an assistant reply.
#[derive(Clone, Debug, PartialEq)]
enum Segment {
    Text(String),
    Image { alt: String, path: String },
}

fn image(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix("![")?;
    let (alt, rest) = rest.split_once("](")?;
    let path = rest.strip_suffix(')')?;
    (!path.is_empty() && !path.contains(' ')).then(|| (alt.to_owned(), path.to_owned()))
}

// Split at paragraph boundaries outside code fences to bound virtual-list realization.
const PIECE: usize = 1400;

// Images occupy separate rows for decode caching and height estimation; tables remain markdown.
fn segments(source: &str) -> Vec<Segment> {
    let lines: Vec<&str> = source.split('\n').collect();
    let mut out = Vec::new();
    let mut text = String::new();
    let mut fence = false;
    let mut index = 0;
    let flush = |text: &mut String, out: &mut Vec<Segment>| {
        if !text.trim().is_empty() {
            out.push(Segment::Text(text.trim_matches('\n').to_owned()));
        }
        text.clear();
    };
    while index < lines.len() {
        let line = lines[index];
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        if !fence {
            if let Some((alt, path)) = image(line) {
                flush(&mut text, &mut out);
                out.push(Segment::Image { alt, path });
                index += 1;
                continue;
            }
            if line.trim().is_empty() && text.len() > PIECE {
                flush(&mut text, &mut out);
                index += 1;
                continue;
            }
        }
        // Headings step down three levels onto the absolute ladder (see the
        // view's `heading_scale(1.0)`): 17, 15 and 13 px in a compact panel.
        let heading = line.len() - line.trim_start_matches('#').len();
        if !fence
            && (1..=6).contains(&heading)
            && line
                .get(heading..)
                .is_some_and(|tail| tail.starts_with(' '))
        {
            text.push_str(&"#".repeat((heading + 3).min(6)));
            text.push_str(line.get(heading..).unwrap_or_default());
        } else {
            text.push_str(line);
        }
        text.push('\n');
        index += 1;
    }
    flush(&mut text, &mut out);
    out
}

// Hold incomplete image/table syntax until its streamed line is complete.
fn settled(text: &str) -> &str {
    let tail = text.rsplit('\n').next().unwrap_or("");
    if tail.trim_start().starts_with("![") || tail.trim_start().starts_with('|') {
        text.get(..text.len() - tail.len()).unwrap_or_default()
    } else {
        text
    }
}

fn decode(path: &str) -> Option<(iced::widget::image::Handle, u32, u32)> {
    let path = std::path::Path::new(path.strip_prefix("file://").unwrap_or(path));
    use std::{io::BufReader, os::unix::fs::OpenOptionsExt};

    if !path.is_absolute() {
        return None;
    }
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 16 << 20 {
        return None;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > 16 << 20 {
        return None;
    }
    let mut reader = raster::ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    let mut limits = raster::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    reader.limits(limits);
    let image = reader.decode().ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    (width > 0 && height > 0).then(|| {
        (
            iced::widget::image::Handle::from_rgba(width, height, image.into_raw()),
            width,
            height,
        )
    })
}

/// How the panel's content column is laid out, for height estimates.
pub(super) const CONTENT_WIDTH: f32 = 368.0;

fn wrapped_lines(text: &str, per_line: usize) -> usize {
    text.lines()
        .map(|line| line.chars().count().div_ceil(per_line).max(1))
        .sum::<usize>()
        .max(1)
}

fn estimate(row: &Row) -> f32 {
    match &row.body {
        Body::User { text, .. } => 30.0 + 19.0 * wrapped_lines(text, 40) as f32,
        Body::Markdown { text, .. } => {
            let code = text
                .lines()
                .filter(|line| line.starts_with("    ") || line.starts_with("```"))
                .count();
            let table = text
                .lines()
                .filter(|line| line.trim_start().starts_with('|'))
                .count();
            12.0 + 19.0 * wrapped_lines(text, 54) as f32
                + 4.0 * text.matches("\n\n").count() as f32
                + 6.0 * code as f32
                + 10.0 * table as f32
        }
        Body::Image { image, .. } => image.as_ref().map_or(40.0, |(_, w, h)| {
            (CONTENT_WIDTH.min(*w as f32) * *h as f32 / *w as f32) + 30.0
        }),
        Body::Thought { text, .. } => {
            if row.open {
                44.0 + 17.0 * wrapped_lines(text, 58) as f32
            } else {
                30.0
            }
        }
        Body::Tool { output, .. } => {
            if row.open {
                110.0 + 15.0 * output.lines().count().min(12) as f32
            } else {
                30.0
            }
        }
        Body::Approval { arguments, .. } => 108.0 + 15.0 * arguments.lines().count().min(6) as f32,
        Body::Note { .. } => 40.0,
        Body::Meta { .. } => 26.0,
    }
}

#[derive(Clone, Debug)]
struct Live {
    reply: String,
    // The reply's rows begin here; earlier rows of the turn are settled.
    start: usize,
    message: u64,
    thinking: Option<(usize, Instant)>,
}

#[derive(Clone, Default)]
pub(crate) struct Transcript {
    pub rows: Vec<Row>,
    heights: Vec<f32>,
    geometry: OnceCell<VariableHeightModel>,
    next: u64,
    live: Option<Live>,
    // The whole reply text of the current turn, for its Copy button.
    said: String,
    images: HashMap<String, Option<(iced::widget::image::Handle, u32, u32)>>,
}

impl Transcript {
    fn key(&mut self, kind: &str) -> String {
        self.next += 1;
        format!("{kind}-{}", self.next)
    }
    fn push(&mut self, kind: &str, body: Body) {
        let key = self.key(kind);
        let row = Row {
            key,
            body,
            open: false,
        };
        self.heights.push(estimate(&row));
        self.geometry.take();
        self.rows.push(row);
    }
    pub(crate) fn clear(&mut self) {
        *self = Self {
            next: self.next,
            ..Self::default()
        };
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    fn image(&mut self, path: &str) -> Option<(iced::widget::image::Handle, u32, u32)> {
        self.images
            .entry(path.to_owned())
            .or_insert_with(|| decode(path))
            .clone()
    }
    pub(crate) fn geometry(&self) -> &VariableHeightModel {
        self.geometry
            .get_or_init(|| VariableHeightModel::new(self.heights.iter().copied()))
    }

    pub(crate) fn measure(&mut self, measurement: RowMeasurement, scroll: f32) -> f32 {
        let Some(height) = self.heights.get_mut(measurement.index) else {
            return scroll;
        };
        *height = measurement.height;
        if let Some(model) = self.geometry.get_mut() {
            enzui_widgets::virtual_list::reconcile_measurements(model, scroll, [measurement.into()])
        } else {
            scroll
        }
    }

    fn reply(&mut self, start: usize, message: u64, text: &str, streaming: bool) {
        let pieces = segments(if streaming { settled(text) } else { text });
        let end = start + pieces.len();
        for (index, piece) in pieces.into_iter().enumerate() {
            let slot = start + index;
            let live = streaming && slot + 1 == end;
            let body = match piece {
                Segment::Text(text) => {
                    if let Some(Row {
                        body:
                            Body::Markdown {
                                text: previous,
                                streaming,
                                ..
                            },
                        ..
                    }) = self.rows.get_mut(slot)
                        && previous.as_ref() == text
                    {
                        *streaming = live;
                        continue;
                    }
                    Body::Markdown {
                        blocks: Rc::default(),
                        text: text.into(),
                        streaming: live,
                    }
                }
                Segment::Image { alt, path } => {
                    if let Some(Row {
                        body:
                            Body::Image {
                                alt: previous_alt,
                                path: previous_path,
                                ..
                            },
                        ..
                    }) = self.rows.get(slot)
                        && *previous_alt == alt
                        && *previous_path == path
                    {
                        continue;
                    }
                    Body::Image {
                        image: self.image(&path),
                        alt,
                        path,
                    }
                }
            };
            let row = Row {
                key: format!("reply-{message}-{index}"),
                body,
                open: false,
            };
            if let Some(previous) = self.rows.get_mut(slot) {
                if std::mem::discriminant(&previous.body) != std::mem::discriminant(&row.body) {
                    self.heights[slot] = estimate(&row);
                    self.geometry.take();
                }
                *previous = row;
            } else {
                self.heights.push(estimate(&row));
                self.rows.push(row);
                self.geometry.take();
            }
        }
        if end < self.rows.len() {
            self.rows.truncate(end);
            self.heights.truncate(end);
            self.geometry.take();
        }
    }
    // A saved chat, as `session.export` lists it.
    pub(crate) fn load(&mut self, items: &[Value]) {
        self.clear();
        for value in items {
            let Some(item) = Item::from_json(value) else {
                continue;
            };
            match item {
                Item::User(text) => self.push(
                    "user",
                    Body::User {
                        text,
                        steering: false,
                        attachments: value["attachments"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect(),
                    },
                ),
                Item::Assistant(text) => {
                    self.next += 1;
                    let message = self.next;
                    let start = self.rows.len();
                    self.reply(start, message, &text, false);
                    if !text.trim().is_empty() {
                        self.push(
                            "meta",
                            Body::Meta {
                                text: String::new(),
                                reply: text,
                            },
                        );
                    }
                }
                Item::Reasoning { text, millis } => self.push(
                    "thought",
                    Body::Thought {
                        text,
                        millis,
                        live: false,
                    },
                ),
                Item::Tool {
                    tool,
                    title,
                    arguments,
                    output,
                    status,
                    millis,
                } => self.push(
                    "tool",
                    Body::Tool {
                        call: String::new(),
                        tool,
                        title,
                        arguments: pretty(&arguments),
                        output,
                        status: match status.as_str() {
                            "failed" => ToolStatus::Failed,
                            "cancelled" => ToolStatus::Cancelled,
                            _ => ToolStatus::Done,
                        },
                        millis,
                    },
                ),
                Item::Note(text) => self.push("note", Body::Note { text, error: false }),
            }
        }
    }
    pub(crate) fn begin(&mut self, prompt: &str, attachments: Vec<String>) {
        self.push(
            "user",
            Body::User {
                text: prompt.to_owned(),
                steering: false,
                attachments,
            },
        );
        self.said.clear();
        self.next += 1;
        self.live = Some(Live {
            reply: String::new(),
            start: self.rows.len(),
            message: self.next,
            thinking: None,
        });
    }
    // What the turn is doing now, for the status line.
    pub(crate) fn activity(&self) -> &'static str {
        let Some(live) = &self.live else { return "" };
        if live.thinking.is_some() {
            return "Thinking";
        }
        match self.rows.last().map(|row| &row.body) {
            Some(Body::Tool {
                status: ToolStatus::Running,
                tool,
                ..
            }) => match tool.as_str() {
                "run_command" => tr("shell-running-a-command"),
                "read_file" => tr("shell-reading-a-file"),
                "write_file" => tr("shell-writing-a-file"),
                "search_files" => tr("shell-searching-files"),
                "render_chart" => tr("shell-rendering-a-chart"),
                _ => tr("shell-using-a-tool"),
            },
            Some(Body::Approval {
                decision: Decision::Pending,
                ..
            }) => tr("shell-waiting-for-your-approval"),
            Some(Body::Markdown {
                streaming: true, ..
            }) => tr("shell-writing"),
            _ => tr("shell-working"),
        }
    }
    fn settle_reply(&mut self) {
        if let Some(live) = self.live.as_mut()
            && !live.reply.is_empty()
        {
            let (start, message, reply) =
                (live.start, live.message, std::mem::take(&mut live.reply));
            self.reply(start, message, &reply, false);
            if !self.said.is_empty() {
                self.said.push_str("\n\n");
            }
            self.said.push_str(&reply);
        }
    }
    fn restart_reply(&mut self) {
        self.settle_reply();
        let start = self.rows.len();
        self.next += 1;
        let message = self.next;
        if let Some(live) = self.live.as_mut() {
            live.start = start;
            live.message = message;
        }
    }
    // Applies one protocol event.
    pub(crate) fn apply(&mut self, event: &Value) {
        let text = |key: &str| event[key].as_str().unwrap_or("").to_owned();
        match event["type"].as_str().unwrap_or("") {
            "reasoning.delta" => {
                let thinking = self.live.as_ref().and_then(|live| live.thinking);
                match thinking {
                    Some((index, _)) => {
                        if let Some(Row {
                            body: Body::Thought { text: current, .. },
                            ..
                        }) = self.rows.get_mut(index)
                        {
                            current.push_str(&text("text"));
                        }
                    }
                    None => {
                        self.restart_reply();
                        self.push(
                            "thought",
                            Body::Thought {
                                text: text("text"),
                                millis: 0,
                                live: true,
                            },
                        );
                        let index = self.rows.len() - 1;
                        if let Some(live) = self.live.as_mut() {
                            live.thinking = Some((index, Instant::now()));
                            live.start = self.rows.len();
                        }
                    }
                }
            }
            "reasoning.completed" => {
                if let Some((index, started)) =
                    self.live.as_mut().and_then(|live| live.thinking.take())
                    && let Some(Row {
                        body: Body::Thought { millis, live, .. },
                        ..
                    }) = self.rows.get_mut(index)
                {
                    *millis = event["duration_ms"]
                        .as_u64()
                        .unwrap_or_else(|| started.elapsed().as_millis() as u64);
                    *live = false;
                }
            }
            "message.delta" => {
                let Some(live) = self.live.as_mut() else {
                    return;
                };
                let mut reply = std::mem::take(&mut live.reply);
                reply.push_str(event["text"].as_str().unwrap_or_default());
                let (start, message) = (live.start, live.message);
                self.reply(start, message, &reply, true);
                if let Some(live) = self.live.as_mut() {
                    live.reply = reply;
                }
            }
            "message.completed" => {
                if let Some(live) = self.live.as_mut()
                    && live.reply.is_empty()
                {
                    live.reply = text("content");
                }
                self.restart_reply();
            }
            "tool.started" => {
                self.restart_reply();
                self.push(
                    "tool",
                    Body::Tool {
                        call: text("call_id"),
                        tool: text("tool"),
                        title: text("title"),
                        arguments: pretty(&event["arguments"]),
                        output: String::new(),
                        status: ToolStatus::Running,
                        millis: 0,
                    },
                );
                self.restart_reply();
            }
            "tool.output.delta" => {
                let call = text("call_id");
                if let Some(Body::Tool { output, .. }) = self.tool(&call) {
                    output.push_str(&text("text"));
                }
            }
            "tool.completed" => {
                let call = text("call_id");
                let millis = event["duration_ms"].as_u64().unwrap_or(0);
                let final_output = text("output");
                let state = match event["status"].as_str() {
                    Some("failed") => ToolStatus::Failed,
                    Some("cancelled") => ToolStatus::Cancelled,
                    _ => ToolStatus::Done,
                };
                if let Some(Body::Tool {
                    status,
                    millis: time,
                    output,
                    ..
                }) = self.tool(&call)
                {
                    *status = state;
                    *time = millis;
                    if !final_output.is_empty() {
                        *output = final_output;
                    }
                }
            }
            "approval.requested" => {
                self.restart_reply();
                self.push(
                    "approval",
                    Body::Approval {
                        request: text("request_id"),
                        binding: text("binding"),
                        tool: text("tool"),
                        title: text("title"),
                        arguments: pretty(&event["arguments"]),
                        decision: Decision::Pending,
                    },
                );
                self.restart_reply();
            }
            "approval.resolved" => {
                let request = text("request_id");
                let allow = event["allow"].as_bool();
                for row in &mut self.rows {
                    if let Body::Approval {
                        request: id,
                        decision,
                        ..
                    } = &mut row.body
                        && *id == request
                    {
                        // A refusal nobody here sent is the request lapsing.
                        *decision = match (allow, *decision) {
                            (Some(true), _) => Decision::Allowed,
                            (Some(false), Decision::Sending(_)) => Decision::Denied,
                            _ => Decision::Closed,
                        };
                    }
                }
            }
            "steer.accepted" => {
                self.restart_reply();
                self.push(
                    "user",
                    Body::User {
                        text: text("text"),
                        steering: true,
                        attachments: Vec::new(),
                    },
                );
                self.restart_reply();
            }
            "error" => {
                self.restart_reply();
                self.push(
                    "note",
                    Body::Note {
                        text: text("message"),
                        error: true,
                    },
                );
            }
            "turn.vision_fallback" => {
                self.restart_reply();
                self.push(
                    "notice",
                    Body::Note {
                        text: tr("shell-assistant-vision-fallback").into(),
                        error: false,
                    },
                );
            }
            "turn.local_limit" => {
                self.restart_reply();
                self.push(
                    "note",
                    Body::Note {
                        text: tr("shell-assistant-local-limit").to_owned(),
                        error: false,
                    },
                );
            }
            "turn.completed" if event["status"] != "failed" => {
                let seconds = event["duration_ms"].as_u64().unwrap_or(0) as f32 / 1000.0;
                let tokens = event["usage"]["output_tokens"].as_u64().unwrap_or(0);
                self.finish(Some(match event["status"].as_str() {
                    Some("cancelled") => tr!("shell-stopped-after-s", "arg0" => enzui_shell::i18n::number(format!("{seconds:.1}"))),
                    _ if tokens > 0 => tr!("shell-s-tokens", "arg0" => enzui_shell::i18n::number(format!("{seconds:.1}")), "arg1" => compact(tokens)),
                    _ => format!("{seconds:.1} s"),
                }));
            }
            _ => {}
        }
    }
    fn tool(&mut self, call: &str) -> Option<&mut Body> {
        self.rows
            .iter_mut()
            .rev()
            .map(|row| &mut row.body)
            .find(|body| matches!(body, Body::Tool { call: id, .. } if id == call))
    }
    // Ends the turn: settles the reply, stops live rows, adds its meta line.
    pub(crate) fn finish(&mut self, meta: Option<String>) {
        if self.live.is_none() {
            return;
        }
        self.settle_reply();
        for row in &mut self.rows {
            match &mut row.body {
                Body::Thought { live, .. } => *live = false,
                Body::Tool {
                    status: status @ ToolStatus::Running,
                    ..
                } => *status = ToolStatus::Cancelled,
                Body::Approval {
                    decision: decision @ (Decision::Pending | Decision::Sending(_)),
                    ..
                } => *decision = Decision::Closed,
                Body::Markdown { streaming, .. } => *streaming = false,
                _ => {}
            }
        }
        self.live = None;
        if let Some(text) = meta.filter(|text| !text.is_empty()) {
            let reply = std::mem::take(&mut self.said);
            self.push("meta", Body::Meta { text, reply });
        }
    }
    // The turn's connection ended without a final event.
    pub(crate) fn fail(&mut self, error: &str) {
        if self.live.is_some() {
            self.restart_reply();
            if !matches!(
                self.rows.last(),
                Some(Row {
                    body: Body::Note { error: true, .. },
                    ..
                })
            ) {
                self.push(
                    "note",
                    Body::Note {
                        text: error.to_owned(),
                        error: true,
                    },
                );
            }
            self.finish(None);
        }
    }
    pub(crate) fn toggle(&mut self, key: &str) {
        if let Some(index) = self.rows.iter().position(|row| row.key == key) {
            self.rows[index].open = !self.rows[index].open;
            self.heights[index] = estimate(&self.rows[index]);
            if let Some(model) = self.geometry.get_mut() {
                model.set_height(index, self.heights[index]);
            }
        }
    }
    pub(crate) fn decide(&mut self, key: &str, allow: bool) -> Option<(String, String)> {
        let row = self.rows.iter_mut().find(|row| row.key == key)?;
        if let Body::Approval {
            request,
            binding,
            decision: decision @ Decision::Pending,
            ..
        } = &mut row.body
        {
            *decision = Decision::Sending(allow);
            Some((request.clone(), binding.clone()))
        } else {
            None
        }
    }
    pub(crate) fn undecided(&mut self, key: &str) {
        if let Some(Row {
            body: Body::Approval { decision, .. },
            ..
        }) = self.rows.iter_mut().find(|row| row.key == key)
        {
            *decision = Decision::Pending;
        }
    }
    pub(crate) fn last_user(&self) -> Option<(&str, &[String])> {
        self.rows.iter().rev().find_map(|row| match &row.body {
            Body::User {
                text, attachments, ..
            } => Some((text.as_str(), attachments.as_slice())),
            _ => None,
        })
    }

    pub(crate) fn replace_last(&mut self, prompt: &str) {
        if let Some(index) = self
            .rows
            .iter()
            .rposition(|row| matches!(row.body, Body::User { .. }))
        {
            let files = self
                .last_user()
                .map(|(_, files)| files.to_vec())
                .unwrap_or_default();
            self.rows.truncate(index);
            self.heights.truncate(index);
            self.geometry.take();
            self.begin(prompt, files);
        }
    }

    // The last user message, for Retry.
    pub(crate) fn last_prompt(&self) -> Option<String> {
        self.rows.iter().rev().find_map(|row| match &row.body {
            Body::User {
                text,
                steering: false,
                ..
            } => Some(text.clone()),
            _ => None,
        })
    }
}

fn compact(tokens: u64) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", tokens as f32 / 1000.0)
    } else {
        tokens.to_string()
    }
}

// Tool arguments as the panel shows them.
pub(super) fn pretty(arguments: &Value) -> String {
    match arguments {
        Value::Null => String::new(),
        Value::Object(map) if map.len() == 1 && map.values().all(Value::is_string) => map
            .values()
            .next()
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        _ => serde_json::to_string_pretty(arguments).unwrap_or_default(),
    }
}

impl ItemProvider<Row> for Transcript {
    fn len(&self) -> u64 {
        self.rows.len() as u64
    }
    fn item(&self, index: u64) -> Option<Row> {
        self.rows.get(usize::try_from(index).ok()?).cloned()
    }
}

impl KeyedProvider<Row> for Transcript {
    fn key(&self, index: u64) -> Option<SequenceKey> {
        self.rows
            .get(usize::try_from(index).ok()?)
            .map(|row| SequenceKey::new(row.key.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn failed_completion_keeps_partial_reply_and_recovery_message() {
        let mut transcript = Transcript::default();
        transcript.begin("Explain enums", vec![]);
        transcript.apply(&json!({"type": "message.delta", "text": "An enum can represent"}));
        transcript.apply(&json!({"type": "turn.completed", "status": "failed"}));
        transcript.fail("The AI gateway is offline");

        assert!(transcript.rows.iter().any(|row| matches!(
            &row.body,
            Body::Markdown { text, streaming: false, .. } if text.as_ref() == "An enum can represent"
        )));
        assert!(matches!(
            transcript.rows.last().map(|row| &row.body),
            Some(Body::Note { text, error: true }) if text == "The AI gateway is offline"
        ));
        assert_eq!(transcript.last_prompt().as_deref(), Some("Explain enums"));
    }
}
