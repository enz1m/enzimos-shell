use super::{
    DURATION, Entry, Message, Phase, Rect, Shell, Spread, SpreadMessage, assign_window_keys, ease,
    spread_places,
};
use iced::Task;
use std::{collections::HashSet, time::Instant};

#[derive(Debug, Default)]
pub(super) struct CloseState {
    pending: HashSet<u64>,
    checking: bool,
    dirty: bool,
    removed: HashSet<u64>,
    reflow: Option<Reflow>,
}

#[derive(Debug)]
struct Reflow {
    from: Vec<Rect>,
    captions: Vec<(f32, f32)>,
}

pub(super) fn caption_line(own: Rect, row: impl Iterator<Item = Rect>) -> f32 {
    let bottom = own.y + own.height;
    let line = row
        .filter(|other| other.y < bottom && own.y < other.y + other.height)
        .map(|other| other.y + other.height)
        .fold(bottom, f32::max);
    if line - bottom > 56.0 { bottom } else { line }
}

impl Spread {
    pub(super) fn live_index(&self, id: u64) -> Option<usize> {
        (!self.close.removed.contains(&id))
            .then(|| {
                self.entries
                    .iter()
                    .position(|entry| entry.window.stable_id == id)
            })
            .flatten()
    }

    pub(super) fn current_tile(&self) -> Option<u64> {
        self.selected
            .filter(|&id| self.live_index(id).is_some())
            .or_else(|| self.hovered.filter(|&id| self.live_index(id).is_some()))
            .or_else(|| {
                self.entries
                    .iter()
                    .rev()
                    .map(|entry| entry.window.stable_id)
                    .find(|id| !self.close.removed.contains(id))
            })
    }

    pub(super) fn is_reflowing(&self) -> bool {
        self.close.reflow.is_some()
    }

    pub(super) fn reflow_progress(&self, at: Instant) -> Option<f32> {
        self.close.reflow.as_ref().map(|_| {
            self.frames.get().map_or(0.0, |frames| {
                ease(at.saturating_duration_since(frames.first).as_secs_f32() / DURATION)
            })
        })
    }

    pub(super) fn tile_motion(&self, index: usize, t: f32) -> Option<(Rect, f32, f32)> {
        let motion = self.close.reflow.as_ref()?;
        let (from, to) = motion.captions[index];
        let destination = self.entries[index].to;
        let opacity = if self
            .close
            .removed
            .contains(&self.entries[index].window.stable_id)
            && destination.width == 0.0
        {
            1.0 - t
        } else {
            1.0
        };
        Some((
            motion.from[index].lerp(destination, t),
            from + (to - from) * t,
            opacity,
        ))
    }

    fn start_reflow(&mut self, at: Instant, reduced: bool) {
        if self.close.reflow.is_some() || self.close.removed.is_empty() {
            return;
        }
        let from: Vec<_> = self.entries.iter().map(|entry| entry.to).collect();
        let kept: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| !self.close.removed.contains(&entry.window.stable_id))
            .map(|entry| entry.from)
            .collect();
        let placed = spread_places(&kept, self.area, self.insets);
        let mut next = placed.iter().copied();
        let mut captions = Vec::with_capacity(self.entries.len());
        for entry in &mut self.entries {
            let old = entry.to;
            let start = caption_line(old, from.iter().copied());
            if self.close.removed.contains(&entry.window.stable_id) {
                entry.to = Rect::new(old.x + old.width / 2.0, old.y + old.height / 2.0, 0.0, 0.0);
                captions.push((start, start));
            } else if let Some(place) = next.next() {
                entry.to = place;
                captions.push((start, caption_line(place, placed.iter().copied())));
            }
        }
        if reduced {
            self.settle_reflow();
        } else {
            self.started = Some(at);
            self.frames.set(None);
            self.close.reflow = Some(Reflow { from, captions });
        }
    }

    fn settle_reflow(&mut self) {
        self.close.reflow = None;
        self.started = None;
        self.frames.set(None);
        self.entries.retain(|entry| {
            if entry.to.width == 0.0 {
                self.close.removed.remove(&entry.window.stable_id);
                self.close.pending.remove(&entry.window.stable_id);
                false
            } else {
                true
            }
        });
        self.selected = self.selected.filter(|&id| self.live_index(id).is_some());
        self.hovered = self.hovered.filter(|&id| self.live_index(id).is_some());
        if self.close.removed.is_empty() {
            assign_window_keys(&mut self.entries);
        }
    }

    pub(super) fn finish_reflow_frame(&mut self, at: Instant) {
        self.settle_reflow();
        self.start_reflow(at, false);
    }

    pub(super) fn freeze_reflow(&mut self, at: Instant) {
        if let Some(t) = self.reflow_progress(at) {
            for index in 0..self.entries.len() {
                if let Some((rect, _, _)) = self.tile_motion(index, t) {
                    self.entries[index].to = rect;
                }
            }
            self.close.reflow = None;
        }
    }

    pub(super) fn entry_gone(&self, entry: &Entry) -> bool {
        self.close.removed.contains(&entry.window.stable_id)
    }
}

impl Shell {
    pub(crate) fn spread_close_prompt_pids(&self) -> Vec<u32> {
        self.spread
            .entries
            .iter()
            .filter(|entry| self.spread.close.pending.contains(&entry.window.stable_id))
            .map(|entry| entry.window.pid)
            .collect()
    }

    pub(super) fn request_spread_close(&mut self, id: u64) -> Task<Message> {
        if !self.activities
            || self.spread.phase != Phase::Open
            || self.spread.live_index(id).is_none()
        {
            return Task::none();
        }
        self.spread.close.pending.insert(id);
        let compositor = self.compositor.clone();
        let generation = self.spread.generation;
        Task::perform(
            async move { compositor.spread_request_close(id).await },
            move |result| Message::Spread(SpreadMessage::CloseRequested(generation, id, result)),
        )
    }

    pub(crate) fn check_spread_windows(&mut self) -> Task<Message> {
        if !self.activities {
            return Task::none();
        }
        if matches!(self.spread.phase, Phase::Loading | Phase::Opening) {
            self.spread.close.dirty = true;
        }
        if self.spread.phase != Phase::Open || self.spread.entries.is_empty() {
            return Task::none();
        }
        if self.spread.close.checking {
            self.spread.close.dirty = true;
            return Task::none();
        }
        self.spread.close.checking = true;
        let generation = self.spread.generation;
        let compositor = self.compositor.clone();
        Task::perform(
            async move { compositor.spread_live_windows().await },
            move |result| Message::Spread(SpreadMessage::LiveWindows(generation, result)),
        )
    }

    pub(super) fn spread_windows_read(
        &mut self,
        result: Result<HashSet<u64>, String>,
    ) -> Task<Message> {
        self.spread.close.checking = false;
        self.quiet = true;
        if self.spread.phase == Phase::Open
            && let Ok(live) = result
        {
            let count = self.spread.close.removed.len();
            self.spread.close.removed.extend(
                self.spread
                    .entries
                    .iter()
                    .map(|entry| entry.window.stable_id)
                    .filter(|id| !live.contains(id)),
            );
            if self.spread.close.removed.len() != count {
                let reduced = self.window_tokens(self.spread.window).motion.reduced_motion;
                self.spread.start_reflow(Instant::now(), reduced);
                self.quiet = false;
            }
        }
        self.check_spread_windows_if_dirty()
    }

    pub(super) fn check_spread_windows_if_dirty(&mut self) -> Task<Message> {
        if std::mem::take(&mut self.spread.close.dirty) {
            self.check_spread_windows()
        } else {
            Task::none()
        }
    }

    pub(super) fn spread_close_requested(
        &mut self,
        id: u64,
        result: Result<(), String>,
    ) -> Task<Message> {
        if let Err(error) = result {
            self.spread.close.pending.remove(&id);
            self.notice = Some(error);
        }
        self.check_spread_windows()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_spread::SpreadWindow;
    use iced::Padding;
    use std::time::Duration;

    #[test]
    fn a_window_stays_until_it_unmaps_and_keys_change_after_reflow() {
        let compositor =
            enzui_shell::compositor::Compositor::new("/usr/bin/true".into(), "fixture".into())
                .unwrap();
        let mut shell = Shell::new(compositor, "/".into()).0;
        shell.activities = true;
        shell.spread.phase = Phase::Open;
        shell.spread.area = [0.0, 0.0, 1440.0, 900.0];
        shell.spread.insets = Padding::ZERO;
        let original = [
            Rect::new(50.0, 300.0, 300.0, 240.0),
            Rect::new(600.0, 300.0, 300.0, 240.0),
            Rect::new(1040.0, 300.0, 300.0, 240.0),
        ];
        shell.spread.entries = original
            .into_iter()
            .enumerate()
            .map(|(index, rect)| {
                let id = index as u64 + 1;
                Entry {
                    window: SpreadWindow {
                        address: format!("0x{id:x}"),
                        stable_id: id,
                        pid: 2,
                        title: format!("Fixture {id}"),
                        class: "fixture".into(),
                        at: [rect.x, rect.y],
                        size: [rect.width, rect.height],
                    },
                    from: rect,
                    to: rect,
                    picture: None,
                    shortcut: None,
                }
            })
            .collect();
        assign_window_keys(&mut shell.spread.entries);
        shell.spread.selected = Some(2);

        let _ = shell.request_spread_close(1);
        let _ = shell.spread_windows_read(Ok(HashSet::from([1, 2, 3])));
        assert!(shell.spread.live_index(1).is_some());
        assert!(!shell.spread.is_reflowing());

        let _ = shell.spread_windows_read(Ok(HashSet::from([2, 3])));
        assert!(shell.spread.live_index(1).is_none());
        assert!(shell.spread.is_reflowing());
        assert_eq!(shell.spread.entries[1].shortcut, Some("2"));
        let (outgoing, _, opacity) = shell.spread.tile_motion(0, 0.5).unwrap();
        assert!(outgoing.width < original[0].width && opacity < 1.0);

        let _ = shell.request_spread_close(3);
        let _ = shell.spread_windows_read(Ok(HashSet::from([2])));
        assert!(shell.spread.live_index(3).is_none());

        let window = iced::window::Id::unique();
        shell.spread.window = Some(window);
        let first = Instant::now();
        assert!(!shell.spread.sample_frame(first));
        let finished = first + Duration::from_secs(1);
        assert!(shell.spread.sample_frame(finished));
        let _ = shell.spread_message(SpreadMessage::MotionFinished(window, finished));
        assert!(shell.spread.is_reflowing());
        assert_eq!(shell.spread.entries[0].shortcut, Some("2"));
        assert!(!shell.spread.sample_frame(finished));
        let settled = finished + Duration::from_secs(1);
        assert!(shell.spread.sample_frame(settled));
        let _ = shell.spread_message(SpreadMessage::MotionFinished(window, settled));
        assert!(!shell.spread.is_reflowing());
        assert_eq!(shell.spread.entries.len(), 1);
        assert_eq!(shell.spread.entries[0].shortcut, Some("1"));
        assert_eq!(shell.spread.current_tile(), Some(2));
        assert!(shell.activities);
    }
}
