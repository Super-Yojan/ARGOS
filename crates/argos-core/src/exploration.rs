//! L4 frontier exploration on Terra's `explore` level.
//!
//! Terra plans the run onboard: it picks a frontier on its occupancy map, drives
//! to it through the waypoint follower and local planner, and ends the run on
//! budget, no frontiers left, stop, takeover, a stale map or a safety hold. This
//! station only starts the run with a budget and shows Terra's status object,
//! which is nested on `autonomy/status` as `exploration`.

use crate::{Result, invalid, valid_token};
use serde_json::{Value, json};

/// Budgets the console offers, in minutes.
pub const EXPLORE_BUDGETS_MIN: &[u32] = &[3, 5, 10];
/// Upper bound this station sends. Terra itself accepts up to a day.
pub const EXPLORE_MAX_BUDGET_SECONDS: f64 = 3600.;

/// `{"level":"explore","token","budget_seconds"}`. Terra rejects an explore
/// request without a budget unless the phone already holds one.
pub fn encode_explore_request(token: &str, budget_seconds: f64) -> Result<Vec<u8>> {
    if !valid_token(token) {
        return Err(invalid("invalid autonomy token"));
    }
    if !valid_explore_budget(budget_seconds) {
        return Err(invalid("invalid exploration budget"));
    }
    serde_json::to_vec(&json!({
        "level": "explore",
        "token": token,
        "budget_seconds": budget_seconds,
    }))
    .map_err(|e| crate::Error(e.to_string()))
}

pub fn valid_explore_budget(seconds: f64) -> bool {
    seconds.is_finite() && seconds > 0. && seconds <= EXPLORE_MAX_BUDGET_SECONDS
}

/// Why the console will not start exploring now, or `None` when it may.
///
/// Terra holds exploration while outdoor GPS is the navigation source and ends
/// the run at once under a latched stop, so neither is worth sending.
pub fn explore_refusal(stopped: bool, outdoor_gps: bool) -> Option<&'static str> {
    if stopped {
        Some("Reset stop before exploring")
    } else if outdoor_gps {
        Some("Switch navigation to Local before exploring")
    } else {
        None
    }
}

/// Terra's `ExplorationStatus`, the fields the console shows.
#[derive(Clone, Debug, PartialEq)]
pub struct ExplorationReport {
    /// `running` or `ended`. Terra omits the object while idle.
    pub phase: String,
    pub elapsed_s: f64,
    pub remaining_s: f64,
    pub budget_s: f64,
    /// Current frontier goal in local metres.
    pub target: Option<(f64, f64)>,
    pub coverage_m2: f64,
    /// Fraction of map cells observed, 0 to 1.
    pub explored_ratio: f64,
    pub end_reason: Option<String>,
    pub frontiers_attempted: u64,
    pub blacklisted: u64,
    /// The arbiter's tick reason, such as `frontier_retry` or `obstacle_blocked`.
    pub reason: Option<String>,
}

impl ExplorationReport {
    pub fn running(&self) -> bool {
        self.phase == "running"
    }
    /// One line for the console: what the run is doing, or how it ended.
    pub fn state_label(&self) -> String {
        if let Some(reason) = self.end_reason.as_deref() {
            return end_reason_label(reason).to_string();
        }
        match self.reason.as_deref() {
            Some("frontier_retry") => "Stuck · retrying another frontier".into(),
            Some("obstacle_blocked") => "Blocked by an obstacle".into(),
            Some("heading_requires_spin_or_replan") => crate::HEADING_HOLD_LABEL.into(),
            _ if self.target.is_none() => "Choosing a frontier".into(),
            _ => "Exploring".into(),
        }
    }
    /// Label and value rows for the console.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = vec![
            ("State", self.state_label()),
            (
                "Time",
                format!(
                    "{} of {} · {} left",
                    clock(self.elapsed_s),
                    clock(self.budget_s),
                    clock(self.remaining_s)
                ),
            ),
            (
                "Target",
                self.target
                    .map(|(x, y)| format!("{x:.1}, {y:.1} m"))
                    .unwrap_or_else(|| "None".into()),
            ),
            (
                "Covered",
                format!(
                    "{:.0} m² free · {:.0}% of map seen",
                    self.coverage_m2,
                    self.explored_ratio * 100.
                ),
            ),
        ];
        if self.frontiers_attempted > 0 {
            rows.push((
                "Frontiers",
                if self.blacklisted > 0 {
                    format!(
                        "{} tried · {} skipped as stuck",
                        self.frontiers_attempted, self.blacklisted
                    )
                } else {
                    format!("{} tried", self.frontiers_attempted)
                },
            ));
        }
        rows
    }
}

/// Terra's `EndReason` spelled for the operator.
pub fn end_reason_label(reason: &str) -> &str {
    match reason {
        "no_frontiers_left" => "Done · no frontiers left",
        "budget_expired" => "Done · time budget used",
        "operator_stop" => "Ended · stopped by operator",
        "operator_takeover" => "Ended · operator changed level",
        "map_stale" => "Ended · map went stale",
        "safety_hold" => "Ended · safety hold",
        "objective_complete" => "Done · objective complete",
        other => other,
    }
}

/// Read the `exploration` object nested on an autonomy status value.
pub fn decode_exploration(status: &Value) -> Option<ExplorationReport> {
    let value = status.get("exploration").filter(|v| v.is_object())?;
    let phase = value["phase"].as_str()?;
    if phase == "idle" {
        return None;
    }
    let number = |key: &str| {
        value[key]
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.)
            .unwrap_or(0.)
    };
    let target = value
        .get("target")
        .filter(|t| t.is_object())
        .and_then(|t| Some((t["x"].as_f64()?, t["y"].as_f64()?)))
        .filter(|(x, y)| x.is_finite() && y.is_finite());
    Some(ExplorationReport {
        phase: phase.to_string(),
        elapsed_s: number("elapsed_seconds"),
        remaining_s: number("remaining_seconds"),
        budget_s: number("budget_seconds"),
        target,
        coverage_m2: number("coverage_m2"),
        explored_ratio: number("explored_ratio").min(1.),
        end_reason: value["end_reason"].as_str().map(str::to_string),
        frontiers_attempted: value["frontiers_attempted"].as_u64().unwrap_or(0),
        blacklisted: value["blacklist_count"].as_u64().unwrap_or(0),
        reason: status["reason"]
            .as_str()
            .filter(|r| !r.is_empty())
            .map(str::to_string),
    })
}

fn clock(seconds: f64) -> String {
    let whole = seconds.max(0.).round() as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}
