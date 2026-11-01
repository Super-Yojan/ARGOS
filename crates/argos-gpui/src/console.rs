//! What the C3 console says about one rover: its field callout, the
//! inspector's trace and sections, and the fleet members without a position.
//! Derived from fleet telemetry only; the window owns selection, stop
//! latches, and drive.

use crate::{RoverMark, RoverPresence};
use argos_core::{
    HEADING_HOLD, HEADING_HOLD_LABEL, OPERATOR_LEVELS, Travel, drive_block, level_label,
    phase_label, pose_warning_text,
};
use std::collections::BTreeSet;

/// Navigation sources the operator can request, as `(wire, label)`.
pub const NAVIGATION_SOURCES: [(&str, &str); 2] =
    [("local", "Local"), ("outdoor_gps", "Outdoor GPS")];

/// Manual drive speed caps offered in the Drive section, in m/s.
pub const DRIVE_SPEEDS: [f64; 5] = [0.1, 0.2, 0.3, 0.4, 0.5];

/// The selected rover's inspector views, in tab order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InspectorTab {
    #[default]
    Overview,
    Video,
}
impl InspectorTab {
    pub const ALL: [Self; 2] = [Self::Overview, Self::Video];
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Video => "Video",
        }
    }
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }
    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }
}

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

/// The control in this window that sent a stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopOrigin {
    /// The inspector's Stop button.
    Button,
    /// The Space key.
    Space,
    /// Stop all on the console bar.
    StopAll,
    /// The ARGOS operator agent's Stop or Stop all.
    Agent,
}
impl StopOrigin {
    pub fn label(self) -> &'static str {
        match self {
            Self::Button => "Stop button",
            Self::Space => "Space key",
            Self::StopAll => "Stop all",
            Self::Agent => "ARGOS agent",
        }
    }
}

/// Why a rover is in a safety stop.
#[derive(Clone, Debug, PartialEq)]
pub enum StopCause {
    /// This window sent the stop, `age_s` seconds ago.
    Operator { origin: StopOrigin, age_s: f64 },
    /// The rover reports a stop this window did not send. `reason` is the
    /// rover's autonomy reason, when it gives one.
    Rover { reason: Option<String> },
}

/// What the operator does to leave any safety stop.
pub const STOP_RECOVERY: &str = "Press Reset stop to resume";

impl StopCause {
    /// One line naming who stopped the rover, for the callout.
    pub fn label(&self) -> String {
        match self {
            Self::Operator {
                origin: StopOrigin::Agent,
                age_s,
            } => format!("Stopped by the ARGOS agent · {}", ago(*age_s)),
            Self::Operator { origin, age_s } => {
                format!("Stopped by you ({}) · {}", origin.label(), ago(*age_s))
            }
            Self::Rover { reason } => {
                format!("Stopped by rover: {}", rover_stop_reason(reason.as_deref()))
            }
        }
    }
}

/// Terra reports `operator_stop` for any stop request it holds. When this
/// window did not send it, it came from another ARGOS window, the phone, or
/// an agent.
fn rover_stop_reason(reason: Option<&str>) -> String {
    match reason.map(str::trim) {
        Some("operator_stop") => "stop request not sent from this window".into(),
        Some(reason) if !reason.is_empty() && reason != "idle" => reason.replace('_', " "),
        _ => "no reason reported".into(),
    }
}

fn ago(age_s: f64) -> String {
    let age = age_s.max(0.);
    if age < 60. {
        format!("{age:.0} s ago")
    } else {
        format!("{:.0} min ago", (age / 60.).floor())
    }
}

/// Why the rover is stopped, if it is. This window's own stop wins: the rover
/// echoes it back as its own `emergency_stop`. `sent` is this window's stop
/// and the window clock when it was sent; `now` is the same clock.
pub fn stop_cause(
    mark: &RoverMark,
    sent: Option<(StopOrigin, f64)>,
    now: f64,
) -> Option<StopCause> {
    if let Some((origin, at)) = sent {
        Some(StopCause::Operator {
            origin,
            age_s: now - at,
        })
    } else if mark.safety == "emergency_stop" {
        Some(StopCause::Rover {
            reason: mark.autonomy_reason.clone(),
        })
    } else {
        None
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
    /// What to do to get moving again, while the rover is stopped.
    pub recovery: Option<&'static str>,
}

/// `stop` is why the rover is stopped (see [`stop_cause`]);
/// `driving` is this window's manual drive of that rover.
pub fn callout(mark: &RoverMark, stop: Option<&StopCause>, driving: bool) -> Callout {
    let attention = attention(mark);
    let stopped = stop.is_some();
    let (tag, state) = if let Some(stop) = stop {
        (Some(CalloutTag::Stopped), stop.label())
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
    } else if let Some(report) = mark
        .exploration
        .as_ref()
        .filter(|_| mark.requested_level.as_deref() == Some("explore"))
    {
        // An exploration hold or end cancels the follower goal; say why
        // rather than reporting the rover as idle.
        let state = if report.running() {
            format!(
                "{} · {:.0} min left",
                report.state_label(),
                (report.remaining_s / 60.).ceil()
            )
        } else {
            report.state_label()
        };
        (report.running().then_some(CalloutTag::Goal), state)
    } else if mark.goal_active {
        (Some(CalloutTag::Goal), "Going to goal".into())
    } else {
        (None, format!("{} · idle", level(mark)))
    };
    Callout {
        tag,
        state,
        attention: attention.filter(|_| !stopped),
        recovery: stopped.then_some(STOP_RECOVERY),
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
        (Some(requested), None) => match mark.autonomy_reason.as_deref() {
            // Terra names what tripped the hold; older builds only report pose quality.
            Some(reason @ "sensor_unhealthy") if let Some(hold) = &mark.sensor_hold => {
                format!("requested {requested} · held: {reason} ({})", hold.label())
            }
            Some(reason @ "sensor_unhealthy")
                if mark.degraded && !mark.degrade_reason.is_empty() =>
            {
                format!(
                    "requested {requested} · held: {reason} ({})",
                    mark.degrade_reason
                )
            }
            Some(reason) if reason != "idle" => format!("requested {requested} · held: {reason}"),
            _ => format!("requested {requested} · not confirmed"),
        },
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
    let mut rows = vec![
        ("Mission", mission),
        ("Goal", goal),
        ("Autonomy", autonomy),
        ("Hardware", hardware),
        ("Pose", pose),
        (
            "Attention",
            attention(mark).unwrap_or_else(|| "Clear".into()),
        ),
    ];
    // A hold of one tick is gone before anyone reads the Autonomy row.
    if let Some(hold) = &mark.sensor_hold {
        rows.push((
            "Sensor hold",
            format!(
                "{} · {:.0} s ago · {} this run",
                hold.label(),
                hold.age_s,
                hold.count
            ),
        ));
    }
    rows
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

/// The console bar's status: this window's latest note, else the feed's
/// last command result.
pub fn status_line<'a>(note: &'a str, command: &'a str) -> &'a str {
    if note.is_empty() { command } else { note }
}

/// Setup problems the operator must see without hovering: the link's
/// reason while it is not healthy, then the map tiles notice.
pub fn field_notices(link: &str, detail: &str, maps: Option<&str>) -> Vec<String> {
    (link != "healthy" && !detail.is_empty())
        .then(|| format!("Link {link} · {detail}"))
        .into_iter()
        .chain(maps.map(str::to_owned))
        .collect()
}

/// Index into [`OPERATOR_LEVELS`] of the level this window last requested.
/// Target search counts as L4, so the one L4 button stays selected.
pub fn level_index(mark: &RoverMark) -> Option<usize> {
    let requested = argos_core::operator_level(mark.requested_level.as_deref()?);
    OPERATOR_LEVELS
        .iter()
        .position(|(wire, _)| *wire == requested)
}

/// Current level and, while a study condition is set, the assigned level.
pub fn level_line(mark: &RoverMark) -> String {
    let current = mark
        .effective_level
        .as_deref()
        .or(mark.requested_level.as_deref())
        .map(level_label)
        .unwrap_or("level unknown");
    let assigned = mark
        .assigned_level
        .as_deref()
        .map(|level| format!(" · study fixed at {}", level_label(level)))
        .unwrap_or_default();
    format!("{current}{assigned}")
}

/// Index into [`NAVIGATION_SOURCES`] of the source the vehicle reports.
pub fn source_index(mark: &RoverMark) -> usize {
    let source = mark
        .outdoor
        .as_ref()
        .map(|report| report.source.as_str())
        .unwrap_or("local");
    NAVIGATION_SOURCES
        .iter()
        .position(|(wire, _)| *wire == source)
        .unwrap_or(0)
}

/// What the navigation source means for goals right now.
pub fn navigation_note(mark: &RoverMark) -> &'static str {
    if mark.app_update_required {
        "Local navigation. Outdoor GPS is unavailable in this runtime."
    } else if mark.outdoor_gps
        && mark
            .outdoor
            .as_ref()
            .is_some_and(|report| report.goal_levels.len() > 1)
    {
        "Outdoor GPS acknowledged. At L2–L5, drag the map marker to send the GPS destination. L3–L5 still avoid obstacles on the local map."
    } else if mark.outdoor_gps {
        "Outdoor GPS acknowledged. Drag the map marker. Release sends the WGS84 goal."
    } else {
        "Local navigation. Outdoor GPS stays off until the vehicle acknowledges it."
    }
}

/// Index into [`DRIVE_SPEEDS`] of the active linear cap.
pub fn speed_index(linear: f64) -> Option<usize> {
    DRIVE_SPEEDS
        .iter()
        .position(|speed| (linear - speed).abs() < 0.001)
}

/// The Hardware section's status and, when motion is blocked, why.
pub fn hardware_lines(mark: &RoverMark, stopped: bool) -> (&str, Option<&str>) {
    if stopped {
        ("Stop active", Some("Use Reset stop before driving"))
    } else {
        (mark.hardware.status_label(), drive_block(&mark.hardware))
    }
}

/// True while the planner path has a segment without a published velocity.
pub fn path_provisional(mark: &RoverMark) -> bool {
    mark.route.len() > 1
        && mark
            .route
            .iter()
            .any(|sample| sample.travel == Travel::Unspecified)
}

/// A fleet member the field cannot place.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Unlocated {
    pub id: u64,
    pub label: String,
    pub detail: String,
}

/// Fleet members with no drawable position, in id order. Each shows as an
/// off-screen tab marked "No position".
pub fn unlocated(marks: &[RoverMark], presence: &[RoverPresence]) -> Vec<Unlocated> {
    presence
        .iter()
        .filter(|row| !marks.iter().any(|mark| mark.id == row.id()))
        .map(|row| Unlocated {
            id: row.id(),
            label: format!("Rover {} · {}", row.id(), row.membership()),
            detail: format!(
                "{} · {}",
                row.position_reason(),
                row.observation_age()
                    .map(|age| format!("Last observation {age:.1} s ago"))
                    .unwrap_or_else(|| "Waiting for telemetry".into())
            ),
        })
        .collect()
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
    if let Some(label) = mark
        .outdoor
        .as_ref()
        .and_then(|report| report.operator_hold_label())
    {
        return Some(label.into());
    }
    if mark.hold.as_deref() == Some(HEADING_HOLD) {
        return Some(HEADING_HOLD_LABEL.into());
    }
    mark.hardware.fault.clone()
}

/// One inspector line for a found object: what it is and where it is in the
/// rover's local map. The rover's GPS estimate is only a trailing note.
pub fn object_line(mark: &crate::ObjectMark) -> String {
    let o = &mark.object;
    let mut label = o.label();
    if let Some(first) = label.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    let gps = match (o.latitude, o.longitude) {
        (Some(latitude), Some(longitude)) => format!(
            " · GPS {latitude:.6}, {longitude:.6}{}",
            o.geo_accuracy_m
                .map(|a| format!(" ±{a:.0} m"))
                .unwrap_or_default()
        ),
        _ => String::new(),
    };
    format!(
        "{label} #{} · x {:.2}, y {:.2} m · seen {}×{gps}",
        o.id, o.x, o.y, o.sightings
    )
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/console_tests.rs"]
mod tests;
