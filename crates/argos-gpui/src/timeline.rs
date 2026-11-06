//! The attention timeline: for each rover, the last few minutes split into
//! neglect (running alone), waiting on the operator, and interaction, plus
//! when it is predicted to need the operator next.
//!
//! The window feeds [`Timeline::observe`] every sync with whether the rover
//! has an open help episode (from the session log's help tracker), whether
//! it is at a manual level, and whether this window is attending to it (the
//! rover is selected). [`Timeline::rows`] returns the rows in queue order:
//! longest wait first, then soonest predicted need, then rovers in service.

use std::collections::{BTreeMap, VecDeque};

/// How much history the timeline keeps, in seconds.
pub const TIMELINE_PAST_S: f64 = 300.;
/// How far ahead it draws predictions, in seconds.
pub const TIMELINE_AHEAD_S: f64 = 600.;
/// A gap in updates longer than this splits a span (rover offline, window asleep).
const SPAN_GAP_S: f64 = 1.0;
/// Pose history used for the speed estimate.
const SPEED_WINDOW_S: f64 = 10.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Running alone without the operator.
    Neglect,
    /// Needs the operator and nobody has turned to it yet.
    Waiting,
    /// The operator is on it: selected while it needs help, or driving it.
    Interaction,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub phase: Phase,
    pub start: f64,
    pub end: f64,
}

/// When a rover is expected to need the operator, in seconds from now.
#[derive(Clone, Debug, PartialEq)]
pub struct Forecast {
    pub in_s: f64,
    pub label: String,
}

/// What the window knows about one rover at one sync.
#[derive(Clone, Debug, Default)]
pub struct Observation {
    pub online: bool,
    /// The open help episode, if the rover is waiting for the operator.
    pub help: Option<String>,
    /// At L0/L1: the operator is the controller.
    pub manual: bool,
    /// Selected in this window (or being driven from it).
    pub attended: bool,
    /// Local position, for the speed estimate.
    pub x: f64,
    pub y: f64,
}

#[derive(Default)]
struct Track {
    spans: Vec<Span>,
    /// The help episode the operator has already turned to.
    attended_help: Option<String>,
    poses: VecDeque<(f64, f64, f64)>,
}

#[derive(Default)]
pub struct Timeline {
    tracks: BTreeMap<u64, Track>,
}

/// One timeline row, clipped to the kept history.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: u64,
    pub spans: Vec<Span>,
    /// The current phase, or `None` if the rover is not reporting.
    pub phase: Option<Phase>,
    /// When the current wait began.
    pub waiting_since: Option<f64>,
    /// When the current phase began.
    pub phase_since: Option<f64>,
    pub forecasts: Vec<Forecast>,
}

impl Timeline {
    pub fn observe(&mut self, now: f64, id: u64, seen: &Observation) {
        let track = self.tracks.entry(id).or_default();
        track.spans.retain(|span| span.end >= now - TIMELINE_PAST_S);
        if !seen.online {
            return;
        }
        track.poses.push_back((now, seen.x, seen.y));
        while track
            .poses
            .front()
            .is_some_and(|(t, _, _)| now - t > SPEED_WINDOW_S)
        {
            track.poses.pop_front();
        }
        let phase = match &seen.help {
            _ if seen.manual => {
                if seen.attended {
                    Phase::Interaction
                } else {
                    // A manual rover nobody is driving is a rover waiting on you.
                    Phase::Waiting
                }
            }
            Some(help) => {
                if seen.attended {
                    track.attended_help = Some(help.clone());
                }
                if track.attended_help.as_ref() == Some(help) {
                    Phase::Interaction
                } else {
                    Phase::Waiting
                }
            }
            None => {
                track.attended_help = None;
                Phase::Neglect
            }
        };
        match track.spans.last_mut() {
            Some(last) if last.phase == phase && now - last.end <= SPAN_GAP_S => last.end = now,
            Some(last) if now - last.end <= SPAN_GAP_S => {
                last.end = now;
                track.spans.push(Span {
                    phase,
                    start: now,
                    end: now,
                });
            }
            _ => track.spans.push(Span {
                phase,
                start: now,
                end: now,
            }),
        }
    }

    /// Forget rovers that left the fleet.
    pub fn retain(&mut self, keep: impl Fn(u64) -> bool) {
        self.tracks.retain(|id, _| keep(*id));
    }

    /// Mean ground speed over the last few seconds, m/s.
    pub fn speed(&self, id: u64) -> Option<f64> {
        let poses = &self.tracks.get(&id)?.poses;
        let (t0, x0, y0) = *poses.front()?;
        let (t1, x1, y1) = *poses.back()?;
        let dt = t1 - t0;
        (dt >= 2.).then(|| (x1 - x0).hypot(y1 - y0) / dt)
    }

    /// Rows in queue order. `forecasts` gives each rover's predicted needs.
    pub fn rows(&self, now: f64, forecasts: impl Fn(u64) -> Vec<Forecast>) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .tracks
            .iter()
            .map(|(id, track)| {
                let current = track
                    .spans
                    .last()
                    .filter(|span| now - span.end <= SPAN_GAP_S);
                let phase = current.map(|span| span.phase);
                // A wait that turned into interaction still started when the wait did.
                let waiting_since = (phase == Some(Phase::Waiting))
                    .then(|| current.map(|span| span.start))
                    .flatten();
                let mut predicted = if phase == Some(Phase::Neglect) {
                    forecasts(*id)
                        .into_iter()
                        .filter(|f| f.in_s >= 0. && f.in_s <= TIMELINE_AHEAD_S)
                        .collect()
                } else {
                    Vec::new()
                };
                predicted.sort_by(|a: &Forecast, b| a.in_s.total_cmp(&b.in_s));
                Row {
                    id: *id,
                    spans: track.spans.clone(),
                    phase,
                    waiting_since,
                    phase_since: current.map(|span| span.start),
                    forecasts: predicted,
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            rank(a)
                .partial_cmp(&rank(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.id.cmp(&b.id))
        });
        rows
    }
}

/// Queue order: (group, time within group).
fn rank(row: &Row) -> (u8, f64) {
    match row.phase {
        Some(Phase::Waiting) => (0, row.waiting_since.unwrap_or(0.)),
        Some(Phase::Neglect) => (1, row.forecasts.first().map_or(f64::INFINITY, |f| f.in_s)),
        Some(Phase::Interaction) => (2, row.phase_since.unwrap_or(0.)),
        None => (3, 0.),
    }
}

/// `m:ss` for timeline labels.
pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.).round() as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/timeline_tests.rs"]
mod tests;
