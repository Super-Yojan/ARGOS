//! What needs the operator, in order: the attention queue behind the right
//! panel. Derived from fleet telemetry and the agent's open recommendations;
//! the window owns only the clock and the tracker's memory.

use crate::RoverMark;
use crate::console::{self, StopCause};
use argos_core::level_label;
use std::collections::BTreeMap;

/// Resolved items older than this drop off the Resolved list.
pub const RESOLVED_WINDOW_S: f64 = 600.;

/// Why a rover is in the queue, most urgent first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    Stopped,
    Hold,
    Question,
    Advice,
}

impl Need {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stopped => "Stopped",
            Self::Hold => "Attention",
            Self::Question => "Question",
            Self::Advice => "Advice",
        }
    }
}

/// One thing waiting on the operator.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Stable across frames, so the wait keeps counting.
    pub key: String,
    pub rover: u64,
    /// Short level chip, such as `L3 Planned`.
    pub level: String,
    pub need: Need,
    /// 1 FYI, 2 soon, 3 now.
    pub urgency: u8,
    pub text: String,
    /// Set for an agent recommendation.
    pub rec_id: Option<String>,
}

/// `L4 Explore + search` becomes `L4 Explore`.
pub fn level_chip(mark: &RoverMark) -> String {
    let label = mark
        .effective_level
        .as_deref()
        .or(mark.requested_level.as_deref())
        .map(level_label)
        .unwrap_or("Level ?");
    label
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Items the rovers themselves raise: stops, holds, and task questions.
/// `stop` is the window's stop cause for a rover (see `console::stop_cause`).
pub fn from_marks(
    marks: &[RoverMark],
    stop: impl Fn(&RoverMark) -> Option<StopCause>,
) -> Vec<Item> {
    let mut items = Vec::new();
    for mark in marks {
        let cause = stop(mark);
        let callout = console::callout(mark, cause.as_ref(), false);
        let item = |need, urgency, text: String| Item {
            key: format!("{}:{need:?}", mark.id),
            rover: mark.id,
            level: level_chip(mark),
            need,
            urgency,
            text,
            rec_id: None,
        };
        if let Some(cause) = &cause {
            items.push(item(Need::Stopped, 3, cause.label()));
        } else if let Some(reason) = callout.attention {
            items.push(item(Need::Hold, 2, reason));
        }
        if let Some(task) = mark
            .task
            .as_ref()
            .filter(|task| task.waiting_for_operator())
        {
            let text = task
                .question
                .clone()
                .unwrap_or_else(|| "The task agent asked you a question".into());
            items.push(item(Need::Question, 2, text));
        }
    }
    items
}

/// Remembers when each item first appeared and what was resolved.
#[derive(Debug, Default)]
pub struct Tracker {
    open: BTreeMap<String, (f64, Item)>,
    resolved: Vec<Resolved>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub rover: u64,
    pub text: String,
    pub waited_s: f64,
    pub at: f64,
}

impl Tracker {
    /// Record this frame's items; anything that left the queue is resolved.
    pub fn update(&mut self, items: &[Item], now: f64) {
        let gone: Vec<String> = self
            .open
            .keys()
            .filter(|key| !items.iter().any(|item| &item.key == *key))
            .cloned()
            .collect();
        for key in gone {
            if let Some((since, item)) = self.open.remove(&key) {
                self.resolved.push(Resolved {
                    rover: item.rover,
                    text: item.text,
                    waited_s: (now - since).max(0.),
                    at: now,
                });
            }
        }
        for item in items {
            self.open
                .entry(item.key.clone())
                .and_modify(|(_, kept)| *kept = item.clone())
                .or_insert_with(|| (now, item.clone()));
        }
        self.resolved.retain(|r| now - r.at <= RESOLVED_WINDOW_S);
    }

    pub fn waited(&self, key: &str, now: f64) -> f64 {
        self.open
            .get(key)
            .map_or(0., |(since, _)| (now - since).max(0.))
    }

    /// Open items, most urgent first, then longest waiting.
    pub fn ranked(&self, now: f64) -> Vec<(Item, f64)> {
        let mut ranked: Vec<_> = self
            .open
            .values()
            .map(|(since, item)| (item.clone(), (now - since).max(0.)))
            .collect();
        ranked.sort_by(|a, b| {
            b.0.urgency
                .cmp(&a.0.urgency)
                .then(b.1.total_cmp(&a.1))
                .then(a.0.rover.cmp(&b.0.rover))
        });
        ranked
    }

    /// Newest first.
    pub fn resolved(&self) -> impl Iterator<Item = &Resolved> {
        self.resolved.iter().rev()
    }

    /// Mean wait of everything open, or `None` when nothing waits.
    pub fn mean_wait(&self, now: f64) -> Option<f64> {
        if self.open.is_empty() {
            return None;
        }
        let total: f64 = self
            .open
            .values()
            .map(|(since, _)| (now - since).max(0.))
            .sum();
        Some(total / self.open.len() as f64)
    }
}

/// `mm:ss`, as the queue shows a wait.
pub fn clock(seconds: f64) -> String {
    let whole = seconds.max(0.).round() as u64;
    format!("{:02}:{:02}", whole / 60, whole % 60)
}

/// Something that will need the operator later.
#[derive(Clone, Debug, PartialEq)]
pub struct Upcoming {
    pub rover: u64,
    pub level: String,
    pub text: String,
    /// Seconds until it needs the operator, when known.
    pub remaining_s: Option<f64>,
    /// Seconds the whole stretch lasts, for the progress bar.
    pub budget_s: Option<f64>,
}

/// Exploration budgets that are running out, and rovers the operator is driving.
pub fn upcoming(marks: &[RoverMark], driving: Option<u64>) -> Vec<Upcoming> {
    let mut rows: Vec<Upcoming> = marks
        .iter()
        .filter_map(|mark| {
            if driving == Some(mark.id) {
                return Some(Upcoming {
                    rover: mark.id,
                    level: level_chip(mark),
                    text: "operator driving now".into(),
                    remaining_s: None,
                    budget_s: None,
                });
            }
            let report = mark
                .exploration
                .as_ref()
                .filter(|report| report.running())?;
            Some(Upcoming {
                rover: mark.id,
                level: level_chip(mark),
                text: "exploration budget ends".into(),
                remaining_s: Some(report.remaining_s),
                budget_s: Some(report.budget_s),
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        a.remaining_s
            .unwrap_or(f64::INFINITY)
            .total_cmp(&b.remaining_s.unwrap_or(f64::INFINITY))
    });
    rows
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/attention_tests.rs"]
mod tests;
