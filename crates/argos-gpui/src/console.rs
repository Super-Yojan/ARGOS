//! What the C3 console says about one rover: its field callout and the
//! inspector's trace. Derived from fleet telemetry only; the window owns
//! selection, stop latches, and drive.

use crate::{RoverMark, RoverPresence};
use argos_core::{HEADING_HOLD, HEADING_HOLD_LABEL, level_label, phase_label, pose_warning_text};
use std::collections::BTreeSet;

/// The one state a callout flags, most urgent first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalloutTag {
    Stopped,
    Driving,
    Attention,
    Goal,
}
impl CalloutTag {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stopped => "Stopped",
            Self::Driving => "Driving",
            Self::Attention => "Attention",
            Self::Goal => "Goal",
        }
    }
}

/// One rover's line on the field.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Callout {
    pub tag: Option<CalloutTag>,
    pub state: String,
    /// Why a person should look, while the rover is not stopped.
    pub attention: Option<String>,
}

/// `stopped` is this window's latch or the rover's own reported stop;
/// `driving` is this window's manual drive of that rover.
pub fn callout(mark: &RoverMark, stopped: bool, driving: bool) -> Callout {
    let attention = attention(mark);
    let (tag, state) = if stopped {
        (Some(CalloutTag::Stopped), "Safety stop active".to_string())
    } else if driving {
        (Some(CalloutTag::Driving), "Manual · you are driving".into())
    } else if let Some(reason) = &attention {
        (Some(CalloutTag::Attention), reason.clone())
    } else if let Some(report) = mark.outdoor.as_ref().filter(|r| r.mission_active()) {
        let remaining = report
            .remaining_m
            .map(|metres| format!(" · {metres:.0} m left"))
            .unwrap_or_default();
        (
            Some(CalloutTag::Goal),
            format!("{}{remaining}", phase_label(&report.phase)),
        )
    } else if mark.goal_active {
        (Some(CalloutTag::Goal), "Going to goal".into())
    } else {
        (None, format!("{} · idle", level(mark)))
    };
    Callout {
        tag,
        state,
        attention: attention.filter(|_| !stopped),
    }
}

/// Inspector rows, in display order.
pub fn trace(mark: &RoverMark, stopped: bool) -> Vec<(&'static str, String)> {
    let mission = match mark.outdoor.as_ref() {
        Some(report) if report.mission_active() || report.phase == "arrived" => {
            phase_label(&report.phase)
        }
        _ if mark.goal_active => "Local goal".into(),
        _ => "None".into(),
    };
    let goal = match (
        &mark.goal,
        mark.outdoor.as_ref().and_then(|r| r.remaining_m),
    ) {
        (Some(goal), Some(metres)) => format!("{} · {metres:.0} m left", goal.state),
        (Some(goal), None) => goal.state.clone(),
        (None, _) => "None".into(),
    };
    let requested = mark.requested_level.as_deref().map(level_label);
    let effective = mark.effective_level.as_deref().map(level_label);
    let autonomy = match (requested, effective) {
        (Some(requested), Some(effective)) if requested == effective => effective.to_string(),
        (Some(requested), Some(effective)) => {
            format!("requested {requested} · effective {effective}")
        }
        (Some(requested), None) => format!("requested {requested} · not confirmed"),
        (None, Some(effective)) => effective.to_string(),
        (None, None) => "Unknown".into(),
    };
    let hardware = if stopped {
        "Stop active".to_string()
    } else {
        mark.hardware.status_label().to_string()
    };
    let pose = pose_warning_text(mark.command_origin, mark.degraded, &mark.degrade_reason)
        .unwrap_or_else(|| mark.status.clone());
    vec![
        ("Mission", mission),
        ("Goal", goal),
        ("Autonomy", autonomy),
        ("Hardware", hardware),
        ("Pose", pose),
        (
            "Attention",
            attention(mark).unwrap_or_else(|| "Clear".into()),
        ),
    ]
}

/// Every rover in the fleet, located or not. Stop all reaches each one.
pub fn fleet_ids(marks: &[RoverMark], presence: &[RoverPresence]) -> BTreeSet<u64> {
    marks
        .iter()
        .map(|mark| mark.id)
        .chain(presence.iter().map(RoverPresence::id))
        .collect()
}

/// Rover cards that share one anchor on the field, stacked in id order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CardGroup {
    /// Projected position of the group's first rover, in field pixels.
    pub x: f32,
    pub y: f32,
    pub ids: Vec<u64>,
    /// The cards sit before the anchor instead of after it.
    pub leading: bool,
    /// The cards hang below the anchor instead of standing above it.
    pub below: bool,
}

/// An axis-aligned rectangle in field pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl FieldRect {
    fn overlaps(self, other: FieldRect) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }
    fn contains(self, other: FieldRect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.x + other.width <= self.x + self.width
            && other.y + other.height <= self.y + self.height
    }
}

/// Field geometry for card placement, all in field pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardSpace {
    /// The widest a card may draw, including its gap from the anchor.
    pub card_width: f32,
    /// One card's height, including its gap from the anchor or the next card.
    pub card_height: f32,
    /// The visible field, at the origin.
    pub field: FieldRect,
    /// A surface drawn over the field, such as the inspector.
    pub avoid: Option<FieldRect>,
}

/// Groups rovers whose cards would overlap, so each one stays readable, then
/// places each group above-trailing, above-leading, below-trailing, or
/// below-leading: the first that stays inside the field and clear of `avoid`.
/// With none clear it keeps above-trailing. `points` are projected positions
/// in field pixels, in any order.
pub fn place_cards(points: &[(u64, f32, f32)], space: CardSpace) -> Vec<CardGroup> {
    let mut points = points.to_vec();
    points.sort_by_key(|(id, _, _)| *id);
    let mut groups: Vec<CardGroup> = Vec::new();
    for (id, x, y) in points {
        if let Some(group) = groups.iter_mut().find(|group| {
            (group.x - x).abs() < space.card_width
                && (group.y - y).abs() < space.card_height * group.ids.len() as f32
        }) {
            group.ids.push(id);
        } else {
            groups.push(CardGroup {
                x,
                y,
                ids: vec![id],
                leading: false,
                below: false,
            });
        }
    }
    for group in &mut groups {
        let height = space.card_height * group.ids.len() as f32;
        let fits = |leading: bool, below: bool| {
            let rect = FieldRect {
                x: if leading {
                    group.x - space.card_width
                } else {
                    group.x
                },
                y: if below { group.y } else { group.y - height },
                width: space.card_width,
                height,
            };
            space.field.contains(rect) && space.avoid.is_none_or(|avoid| !avoid.overlaps(rect))
        };
        if let Some((leading, below)) = [(false, false), (true, false), (false, true), (true, true)]
            .into_iter()
            .find(|(leading, below)| fits(*leading, *below))
        {
            group.leading = leading;
            group.below = below;
        }
    }
    groups
}

fn level(mark: &RoverMark) -> &str {
    mark.effective_level
        .as_deref()
        .or(mark.requested_level.as_deref())
        .map(level_label)
        .unwrap_or("Level unknown")
}

fn attention(mark: &RoverMark) -> Option<String> {
    if mark.status == "offline" {
        return Some("Offline".into());
    }
    if mark.status == "pose stale" && mark.requested_level.as_deref() != Some("teleop") {
        return Some("Pose stale".into());
    }
    if let Some(label) = mark.outdoor.as_ref().and_then(|report| report.hold_label()) {
        return Some(label.into());
    }
    if mark.hold.as_deref() == Some(HEADING_HOLD) {
        return Some(HEADING_HOLD_LABEL.into());
    }
    mark.hardware.fault.clone()
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/console_tests.rs"]
mod tests;
