//! Arrival alerts: tell the operator when a rover at L2 or L3 reaches its goal
//! and sits idle waiting for the next one.
//!
//! In the 2026-11-06 two-rover run (log 3866429a), rovers that arrived sat
//! idle 9–17 s on average before the operator noticed, 450 s in total and the
//! largest wait cause. An alert opens when the help tracker raises a `done`
//! episode at a goal level. It plays a chime, flashes the rover's card, and
//! adds the rover to the "Needs next goal" queue on the map. The flash stops
//! once the operator selects the rover; the queue entry stays until the rover
//! has work again (the help episode clears). One reminder chime plays if
//! nobody has turned to it after [`REMIND_AFTER_S`].
//!
//! Every step is a `kind: "arrival_alert"` session log record carrying the
//! episode's `help_id`, so the analysis can tell alerted waits from old ones.
//! `ARGOS_ARRIVAL_ALERT=on|silent|off` (default `on`) picks the cue, so a
//! study can run with and without it; the choice is logged at start.

use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Levels where the rover drives to an operator-set goal and then waits:
/// L2 Go to point and L3 Planned.
pub const GOAL_LEVELS: [&str; 2] = ["waypoint_direct", "waypoint"];

/// A second chime when an arrival has waited this long unseen (seconds).
/// The 3866429a median wait was 9–13 s.
pub const REMIND_AFTER_S: f64 = 15.;

/// Card flash period while an alert is unseen (seconds per on/off cycle).
pub const FLASH_PERIOD_S: f64 = 1.;

/// Which cues an arrival gets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cue {
    /// Chime, card flash and queue entry.
    #[default]
    On,
    /// Card flash and queue entry, no sound.
    Silent,
    /// Nothing; arrivals are only logged by the help tracker, as before.
    Off,
}

impl Cue {
    /// `on` (default), `silent` or `off`. Anything else means `on`.
    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "off" | "0" | "false" | "no" => Self::Off,
            "silent" | "visual" | "quiet" => Self::Silent,
            _ => Self::On,
        }
    }
    pub fn from_env() -> Self {
        std::env::var("ARGOS_ARRIVAL_ALERT")
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Silent => "silent",
            Self::Off => "off",
        }
    }
    fn cues(self) -> &'static [&'static str] {
        match self {
            Self::On => &["sound", "flash", "queue"],
            Self::Silent => &["flash", "queue"],
            Self::Off => &[],
        }
    }
}

/// What the window knows about one rover at one sync.
#[derive(Clone, Copy, Debug, Default)]
pub struct Seen<'a> {
    /// The open help episode, if any.
    pub help_id: Option<&'a str>,
    /// The episode's class is `done`: the rover finished and wants a command.
    pub done: bool,
    /// The level the episode was raised at.
    pub level: Option<&'a str>,
    /// The rover is selected in this window.
    pub selected: bool,
}

/// One open arrival alert.
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub help_id: String,
    pub level: String,
    /// Session seconds when the alert opened.
    pub since: f64,
    /// When the operator first selected the rover after the alert.
    pub seen_at: Option<f64>,
    reminded: bool,
}

impl Alert {
    pub fn unseen(&self) -> bool {
        self.seen_at.is_none()
    }
    /// Whether an unseen card is in the bright half of its flash at `now`.
    pub fn flash_on(&self, now: f64) -> bool {
        self.unseen() && ((now - self.since) / (FLASH_PERIOD_S / 2.)).floor() as i64 % 2 == 0
    }
}

/// What one observation asks the window to do.
#[derive(Debug, Default, PartialEq)]
pub struct Update {
    /// Session log records, without time stamps.
    pub records: Vec<Value>,
    /// Play the arrival chime.
    pub chime: bool,
}

/// Arrival alerts per rover.
#[derive(Default)]
pub struct ArrivalAlerts {
    cue: Cue,
    alerts: BTreeMap<u64, Alert>,
    /// A rover just chosen from the queue; its next selection counts as `queue`.
    from_queue: Option<u64>,
    config_logged: bool,
}

impl ArrivalAlerts {
    pub fn new(cue: Cue) -> Self {
        Self {
            cue,
            ..Self::default()
        }
    }

    pub fn cue(&self) -> Cue {
        self.cue
    }

    /// The `config` record, once per session.
    pub fn take_config_record(&mut self) -> Option<Value> {
        (!std::mem::replace(&mut self.config_logged, true)).then(|| {
            json!({
                "kind": "arrival_alert",
                "event": "config",
                "cue": self.cue.name(),
                "levels": GOAL_LEVELS,
                "remind_after_s": REMIND_AFTER_S,
            })
        })
    }

    /// Feed one rover's state at `now` (session seconds).
    pub fn observe(&mut self, now: f64, id: u64, seen: &Seen) -> Update {
        let mut update = Update::default();
        if self.cue == Cue::Off {
            return update;
        }
        let arrived = seen
            .help_id
            .filter(|_| seen.done)
            .zip(seen.level.filter(|level| GOAL_LEVELS.contains(level)));
        if let Some(alert) = self.alerts.get(&id)
            && arrived.is_none_or(|(help_id, _)| help_id != alert.help_id)
        {
            update.records.push(json!({
                "kind": "arrival_alert",
                "event": "closed",
                "rover_id": id,
                "help_id": alert.help_id,
                "level": alert.level,
                "open_s": now - alert.since,
                "seen_after_s": alert.seen_at.map(|at| at - alert.since),
            }));
            self.alerts.remove(&id);
        }
        let from_queue = self.from_queue == Some(id);
        let cue = self.cue;
        let mut opened = None;
        match (self.alerts.get_mut(&id), arrived) {
            (None, Some((help_id, level))) => {
                update.chime = cue == Cue::On;
                update.records.push(json!({
                    "kind": "arrival_alert",
                    "event": "shown",
                    "rover_id": id,
                    "help_id": help_id,
                    "level": level,
                    "cues": cue.cues(),
                    "selected": seen.selected,
                }));
                opened = Some(Alert {
                    help_id: help_id.to_owned(),
                    level: level.to_owned(),
                    since: now,
                    // Already on the rover: no flash, still queued.
                    seen_at: seen.selected.then_some(now),
                    reminded: false,
                });
            }
            (Some(alert), Some(_)) if alert.unseen() && seen.selected => {
                alert.seen_at = Some(now);
                update.records.push(json!({
                    "kind": "arrival_alert",
                    "event": "seen",
                    "rover_id": id,
                    "help_id": alert.help_id,
                    "via": if from_queue { "queue" } else { "select" },
                    "after_s": now - alert.since,
                }));
            }
            (Some(alert), Some(_))
                if alert.unseen() && !alert.reminded && now - alert.since >= REMIND_AFTER_S =>
            {
                alert.reminded = true;
                update.chime = cue == Cue::On;
                update.records.push(json!({
                    "kind": "arrival_alert",
                    "event": "reminded",
                    "rover_id": id,
                    "help_id": alert.help_id,
                    "after_s": now - alert.since,
                }));
            }
            _ => {}
        }
        if let Some(alert) = opened {
            self.alerts.insert(id, alert);
        }
        if from_queue && seen.selected {
            self.from_queue = None;
        }
        update
    }

    /// The operator chose this rover from the "Needs next goal" queue.
    pub fn picked_from_queue(&mut self, id: u64) {
        self.from_queue = Some(id);
    }

    /// Forget rovers that left the fleet.
    pub fn retain(&mut self, keep: impl Fn(u64) -> bool) {
        self.alerts.retain(|id, _| keep(*id));
    }

    pub fn alert(&self, id: u64) -> Option<&Alert> {
        self.alerts.get(&id)
    }

    /// Open alerts, longest waiting first.
    pub fn queue(&self) -> Vec<(u64, &Alert)> {
        let mut queue: Vec<_> = self.alerts.iter().map(|(id, alert)| (*id, alert)).collect();
        queue.sort_by(|a, b| a.1.since.total_cmp(&b.1.since).then(a.0.cmp(&b.0)));
        queue
    }
}

/// Play the arrival chime without blocking. `ARGOS_ARRIVAL_SOUND` may name a
/// sound file; otherwise macOS plays Glass and Linux the freedesktop
/// "complete" sound. Failures are ignored: the flash and queue still show.
pub fn play_chime() {
    std::thread::spawn(|| {
        let custom = std::env::var("ARGOS_ARRIVAL_SOUND").ok();
        let (program, default) = if cfg!(target_os = "macos") {
            ("afplay", "/System/Library/Sounds/Glass.aiff")
        } else if cfg!(target_os = "linux") {
            (
                "paplay",
                "/usr/share/sounds/freedesktop/stereo/complete.oga",
            )
        } else {
            return;
        };
        let file = custom.as_deref().unwrap_or(default);
        let _ = std::process::Command::new(program)
            .arg(file)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/arrival_tests.rs"]
mod tests;
