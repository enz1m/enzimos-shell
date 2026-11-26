//! The chat transcript as virtual-list rows: what the service sent, split
//! into rows the panel draws and the list can measure.

use enzui_core::virtual_sequence::{ItemProvider, KeyedProvider, SequenceKey};
use enzui_shell::agent_mock::Item;
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde_json::Value;
use std::collections::HashMap;
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
    },
    Markdown {
        text: String,
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
    if !path.is_absolute() || std::fs::metadata(path).ok()?.len() > 16 << 20 {
        return None;
    }
    let image = raster::open(path).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    (width > 0 && height > 0 && width <= 8192 && height <= 8192).then(|| {
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

#[derive(Default)]
pub(crate) struct Transcript {
    pub rows: Vec<Row>,
    pub heights: Vec<f32>,
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
        self.rows.push(row);
    }
    pub(crate) fn clear(&mut self) {
        let images = std::mem::take(&mut self.images);
        *self = Self {
            images,
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
    fn reply(&mut self, start: usize, message: u64, text: &str, streaming: bool) {
        let mut rows = Vec::new();
        let pieces = segments(if streaming { settled(text) } else { text });
        let last = pieces.len().saturating_sub(1);
        for (index, piece) in pieces.into_iter().enumerate() {
            let body = match piece {
                Segment::Text(text) => Body::Markdown {
                    text,
                    streaming: streaming && index == last,
                },
                Segment::Image { alt, path } => Body::Image {
                    image: self.image(&path),
                    alt,
                    path,
                },
            };
            rows.push(Row {
                key: format!("reply-{message}-{index}"),
                body,
                open: false,
            });
        }
        let old: HashMap<String, f32> = self.rows[start..]
            .iter()
            .zip(&self.heights[start..])
            .map(|(row, height)| (row.key.clone(), *height))
            .collect();
        self.rows.truncate(start);
        self.heights.truncate(start);
        for row in rows {
            // A growing piece keeps its measured height until measured again.
            let height = old
                .get(&row.key)
                .copied()
                .unwrap_or_else(|| estimate(&row))
                .max(estimate(&row).min(60.0));
            self.heights.push(height);
            self.rows.push(row);
        }
    }
    // A saved chat, as `session.export` lists it.
    pub(crate) fn load(&mut self, items: &[Value]) {
        self.clear();
        for item in items.iter().filter_map(Item::from_json) {
            match item {
                Item::User(text) => self.push(
                    "user",
                    Body::User {
                        text,
                        steering: false,
                    },
                ),
                Item::Assistant(text) => {
                    self.next += 1;
                    let message = self.next;
                    let start = self.rows.len();
                    self.reply(start, message, &text, false);
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
    // The user's message opens a turn.
    pub(crate) fn begin(&mut self, prompt: &str) {
        self.push(
            "user",
            Body::User {
                text: prompt.to_owned(),
                steering: false,
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
                live.reply.push_str(&text("text"));
                let (start, message, reply) = (live.start, live.message, live.reply.clone());
                self.reply(start, message, &reply, true);
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
                self.push("notice", Body::Note { text: tr("shell-assistant-vision-fallback").into(), error: false });
            }
            "turn.local_limit" => {
                self.push(
                    "note",
                    Body::Note {
                        text: tr("shell-assistant-local-limit").to_owned(),
                        error: false,
                    },
                );
            }
            "turn.completed" => {
                let seconds = event["duration_ms"].as_u64().unwrap_or(0) as f32 / 1000.0;
                let tokens = event["usage"]["output_tokens"].as_u64().unwrap_or(0);
                self.finish(Some(match event["status"].as_str() {
                    Some("cancelled") => tr!("shell-stopped-after-s", "arg0" => enzui_shell::i18n::number(format!("{seconds:.1}"))),
                    Some("failed") => String::new(),
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
            self.push(
                "note",
                Body::Note {
                    text: error.to_owned(),
                    error: true,
                },
            );
            self.finish(None);
        }
    }
    pub(crate) fn toggle(&mut self, key: &str) {
        if let Some(index) = self.rows.iter().position(|row| row.key == key) {
            self.rows[index].open = !self.rows[index].open;
            self.heights[index] = estimate(&self.rows[index]);
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
    // The last user message, for Retry.
    pub(crate) fn last_prompt(&self) -> Option<String> {
        self.rows.iter().rev().find_map(|row| match &row.body {
            Body::User {
                text,
                steering: false,
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
    fn transcript_behaviour() {
        // Replies split into text and images outside code.
        {
            let source = "Intro\n\n![chart](/tmp/c.png)\n\n| A | B |\n|:--|--:|\n| 1 | 2 |\n\n```\n| not | a table |\n|---|---|\n![no](/x.png)\n```\nEnd";
            let pieces = segments(source);
            assert_eq!(pieces.len(), 3);
            assert_eq!(pieces[0], Segment::Text("Intro".into()));
            assert_eq!(
                segments("## Title\nx"),
                [Segment::Text("##### Title\nx".into())]
            );
            assert_eq!(settled("Text\n![chart](/run/use"), "Text\n");
            assert_eq!(
                pieces[1],
                Segment::Image {
                    alt: "chart".into(),
                    path: "/tmp/c.png".into()
                }
            );
            assert!(
                matches!(&pieces[2], Segment::Text(text) if text.starts_with("| A | B |") && text.contains("| not |") && text.ends_with("End"))
            );
            let long = "word ".repeat(400) + "\n\n" + &"more ".repeat(10);
            assert_eq!(
                segments(&long).len(),
                2,
                "long text is split at a paragraph break"
            );
        }
        // Streamed turn keeps reply keys and settles live rows.
        {
            let mut transcript = Transcript::default();
            transcript.begin("hi");
            transcript.apply(&json!({"type": "reasoning.delta", "text": "Let me"}));
            transcript.apply(&json!({"type": "reasoning.delta", "text": " think"}));
            assert_eq!(transcript.activity(), "Thinking");
            transcript.apply(&json!({"type": "reasoning.completed", "duration_ms": 900}));
            transcript.apply(&json!({"type": "message.delta", "text": "Hel"}));
            let key = transcript.rows.last().unwrap().key.clone();
            transcript.apply(&json!({"type": "message.delta", "text": "lo"}));
            assert_eq!(
                transcript.rows.last().unwrap().key,
                key,
                "the growing piece keeps its key"
            );
            assert!(
                matches!(&transcript.rows.last().unwrap().body, Body::Markdown { text, streaming: true } if text == "Hello")
            );
            transcript.apply(&json!({"type": "message.completed", "content": "Hello"}));
            transcript.apply(&json!({"type": "tool.started", "call_id": "c1", "tool": "run_command", "title": "ls", "arguments": {"command": "ls"}}));
            assert_eq!(transcript.activity(), "Running a command");
            transcript.apply(&json!({"type": "tool.output.delta", "call_id": "c1", "text": "a\n"}));
            transcript.apply(&json!({"type": "approval.requested", "request_id": "r1", "binding": "b", "tool": "write_file", "title": "Write", "arguments": {"path": "/x"}}));
            let approval = transcript.rows.last().unwrap().key.clone();
            assert_eq!(
                transcript.decide(&approval, true),
                Some(("r1".into(), "b".into()))
            );
            transcript
                .apply(&json!({"type": "approval.resolved", "request_id": "r1", "allow": true}));
            transcript.apply(&json!({"type": "turn.completed", "status": "completed", "duration_ms": 4200, "usage": {"output_tokens": 1400}}));
            assert!(transcript.live.is_none());
            let kinds: Vec<&str> = transcript
                .rows
                .iter()
                .map(|row| match row.body {
                    Body::User { .. } => "user",
                    Body::Thought {
                        live: false,
                        millis: 900,
                        ..
                    } => "thought",
                    Body::Markdown {
                        streaming: false, ..
                    } => "text",
                    Body::Tool {
                        status: ToolStatus::Cancelled,
                        ..
                    } => "tool",
                    Body::Approval {
                        decision: Decision::Allowed,
                        ..
                    } => "approval",
                    Body::Meta {
                        ref text,
                        ref reply,
                    } if text.replace(['\u{2068}', '\u{2069}'], "") == "4.2 s · 1.4k tokens"
                        && reply == "Hello" =>
                    {
                        "meta"
                    }
                    _ => "other",
                })
                .collect();
            assert_eq!(
                kinds,
                ["user", "thought", "text", "tool", "approval", "meta"]
            );
            assert_eq!(transcript.heights.len(), transcript.rows.len());
        }
    }
}
