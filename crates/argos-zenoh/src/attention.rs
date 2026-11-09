//! When each rover needs the operator, for wait, interaction and neglect time.
//!
//! Terra's `autonomy/status` says what a rover is doing about 9 times a
//! second. [`HelpTracker`] turns that into debounced help episodes: a
//! `help` record with `event: "raised"` when a rover at L2–L5 stops working
//! (it finished, is blocked, or was stopped), and `event: "cleared"` when it
//! works again or the operator takes it over. Commands sent while an episode
//! is open carry its `help_id`, so the analysis can link the wait, the
//! operator's commands and the resume without guessing.

use serde_json::{Value, json};
use std::collections::BTreeMap;

/// A state must last this long before it counts. Terra flickers between
/// hold reasons and `active` several times a second while replanning.
pub const HELP_DEBOUNCE_S: f64 = 1.5;

/// Levels where the operator drives; the rover never waits for help there.
const MANUAL_LEVELS: [&str; 2] = ["teleop", "assisted_teleop"];

/// Reasons that mean the rover finished and wants a new command.
const DONE_REASONS: [&str; 2] = ["idle", "task_idle"];

/// Reasons that mean the rover is doing its job without the operator.
const WORKING_REASONS: [&str; 19] = [
    "active",
    "assisted",
    "driving_waypoint",
    "exploration_scan",
    "route_planning",
    "search_started",
    "search_unobserved_sector",
    "agent_driving",
    "awaiting_model",
    "awaiting_agent",
    "dock_lock",
    "dock_creep",
    "dock_settle",
    "dock_aligned",
    "dock_closing",
    "dock_dumping",
    "dock_backoff",
    "dock_reacquire",
    "dock_open_range",
];

/// What one status report means for the operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Working on its own: neglect time.
    Working,
    /// Waiting for the operator: wait plus interaction time.
    Help,
    /// The operator drives (L0/L1).
    Manual,
}

/// Help classes: `done` (wants a new command), `blocked` (failed or stuck),
/// `stopped` (emergency stop or paused).
pub fn classify(status: &Value) -> (Phase, Option<&'static str>) {
    let level = status["requested_level"]
        .as_str()
        .or_else(|| status["effective_level"].as_str());
    let reason = status["reason"].as_str().unwrap_or("");
    if status["safety"] == "emergency_stop" {
        return (Phase::Help, Some("stopped"));
    }
    if level.is_some_and(|level| MANUAL_LEVELS.contains(&level)) {
        return (Phase::Manual, None);
    }
    if status["paused"] == true {
        return (Phase::Help, Some("stopped"));
    }
    let source_active = status["active_source"]
        .as_str()
        .is_some_and(|source| source != "none");
    if WORKING_REASONS.contains(&reason) || (source_active && !DONE_REASONS.contains(&reason)) {
        return (Phase::Working, None);
    }
    if DONE_REASONS.contains(&reason) {
        return (Phase::Help, Some("done"));
    }
    (Phase::Help, Some("blocked"))
}

/// A help episode that is still open.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenHelp {
    pub help_id: String,
    /// `done`, `blocked` or `stopped` (see [`classify`]).
    pub class: &'static str,
    /// The level the rover was at when it raised the episode.
    pub level: Option<String>,
    /// Session seconds when it was raised.
    pub raised: f64,
}

#[derive(Default)]
struct Rover {
    phase: Option<Phase>,
    /// A different phase seen since this time, not yet confirmed.
    pending: Option<(Phase, f64)>,
    /// The open episode.
    open: Option<OpenHelp>,
    count: u64,
}

/// Debounced help episodes per rover.
#[derive(Default)]
pub struct HelpTracker {
    rovers: BTreeMap<u64, Rover>,
}

impl HelpTracker {
    /// Feed one `autonomy/status` payload seen at `now` (session seconds).
    /// Returns the `help` records to write, without time stamps.
    pub fn observe(&mut self, id: u64, status: &Value, now: f64) -> Vec<Value> {
        let (phase, class) = classify(status);
        let rover = self.rovers.entry(id).or_default();
        let confirmed = match rover.phase {
            None => true,
            Some(current) if current == phase => {
                rover.pending = None;
                false
            }
            Some(_) => match rover.pending {
                Some((waiting, since)) if waiting == phase => now - since >= HELP_DEBOUNCE_S,
                _ => {
                    rover.pending = Some((phase, now));
                    false
                }
            },
        };
        if !confirmed {
            return Vec::new();
        }
        let since = rover.pending.take().map_or(now, |(_, since)| since);
        rover.phase = Some(phase);
        let mut records = Vec::new();
        if let Some(open) = rover.open.take() {
            records.push(json!({
                "kind": "help",
                "event": "cleared",
                "help_id": open.help_id,
                "rover_id": id,
                "outcome": if phase == Phase::Manual { "to_manual" } else { "resumed" },
                "at": since,
                "duration_s": since - open.raised,
            }));
        }
        if phase == Phase::Help {
            rover.count += 1;
            let help_id = format!("h-{id}-{}", rover.count);
            let level = status["requested_level"]
                .as_str()
                .or_else(|| status["effective_level"].as_str());
            rover.open = Some(OpenHelp {
                help_id: help_id.clone(),
                class: class.unwrap_or("blocked"),
                level: level.map(str::to_owned),
                raised: since,
            });
            records.push(json!({
                "kind": "help",
                "event": "raised",
                "help_id": help_id,
                "rover_id": id,
                "class": class,
                "reason": status["reason"],
                "safety": status["safety"],
                "level": status["requested_level"].as_str().or_else(|| status["effective_level"].as_str()),
                "at": since,
            }));
        }
        records
    }

    /// The help episode open for this rover, if any.
    pub fn open(&self, id: u64) -> Option<String> {
        self.episode(id).map(|open| open.help_id)
    }

    /// The open help episode with its class and level, if any.
    pub fn episode(&self, id: u64) -> Option<OpenHelp> {
        self.rovers.get(&id).and_then(|rover| rover.open.clone())
    }
}

#[cfg(test)]
#[path = "../../../tests/argos-zenoh/unit/attention_tests.rs"]
mod tests;
