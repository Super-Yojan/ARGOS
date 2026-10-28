//! Terra autonomy levels on `<prefix>/<id>/autonomy` and `autonomy/status`.
//!
//! The wire names match Terra's `Level` and the levels `operator_command` accepts.
//! `explore` needs a budget, so it is sent with [`crate::encode_explore_request`].
//! `task` (L5) needs a task, so it is sent with [`crate::encode_task_level_request`].
//! `assigned_level` on the status is the study condition (see [`encode_assignment_request`]).
//! Terra rejects other levels while it is set, except L0 takeover.

use crate::{Result, invalid, valid_token};
use serde_json::{Value, json};

/// Levels the operator station can publish. Wire values are Terra's `Level`.
///
/// L0 `teleop`, L1 `assisted_teleop`, L2 `waypoint_direct` (no avoidance),
/// L3 `waypoint` (local planner), L4 `explore` (frontier exploration on a time
/// budget), L5 `task` (the phone's language model picks waypoints for a typed
/// task). Target search is the other L4 mode. Terra's `supervised` level is
/// not part of the design and has no button; the wire still accepts it.
pub const OPERATOR_LEVELS: &[(&str, &str)] = &[
    ("teleop", "L0 Manual"),
    ("assisted_teleop", "L1 Assisted"),
    ("waypoint_direct", "L2 Direct"),
    ("waypoint", "L3 Waypoint"),
    ("explore", "L4 Explore"),
    ("target_search", "L4 Search"),
    ("task", "L5 Task"),
];

pub fn level_label(level: &str) -> &str {
    OPERATOR_LEVELS
        .iter()
        .find(|(wire, _)| *wire == level)
        .map(|(_, label)| *label)
        .unwrap_or(level)
}

/// `{"level","token"}` with no budget. Terra's `decode_level` accepts this.
/// `explore` is refused here because it needs a budget.
pub fn encode_level_request(level: &str, token: &str) -> Result<Vec<u8>> {
    if level == "explore" {
        return Err(invalid("explore needs a budget"));
    }
    if level == "task" {
        return Err(invalid("L5 needs a task"));
    }
    if !OPERATOR_LEVELS.iter().any(|(wire, _)| *wire == level) {
        return Err(invalid("unsupported autonomy level"));
    }
    if !valid_token(token) {
        return Err(invalid("invalid autonomy token"));
    }
    serde_json::to_vec(&json!({"level": level, "token": token}))
        .map_err(|e| crate::Error(e.to_string()))
}

/// Levels a study condition can fix, in picker order. Supervised is not a
/// study level, and Search runs inside a fixed L4.
pub const STUDY_LEVELS: &[(&str, &str)] = &[
    ("teleop", "L0"),
    ("assisted_teleop", "L1"),
    ("waypoint_direct", "L2"),
    ("waypoint", "L3"),
    ("explore", "L4"),
    ("task", "L5"),
];

/// Whether an operator may select `level` under study condition `assigned`.
/// Mirrors Terra: L0 takeover is always allowed, and a fixed L4 also allows
/// target search.
pub fn level_permitted(assigned: Option<&str>, level: &str) -> bool {
    match assigned {
        None => true,
        Some(assigned) => {
            level == "teleop"
                || level == assigned
                || (assigned == "explore" && level == "target_search")
        }
    }
}

/// `{"assigned_level": <level> | null, "token"}` on the `autonomy` topic.
/// Terra then rejects other levels, and drops to L0 if its current level is
/// outside the condition.
pub fn encode_assignment_request(level: Option<&str>, token: &str) -> Result<Vec<u8>> {
    if level.is_some_and(|level| !STUDY_LEVELS.iter().any(|(wire, _)| *wire == level)) {
        return Err(invalid("not a study level"));
    }
    if !valid_token(token) {
        return Err(invalid("invalid autonomy token"));
    }
    serde_json::to_vec(&json!({"assigned_level": level, "token": token}))
        .map_err(|e| crate::Error(e.to_string()))
}

/// How one planned sample is traversed. Provisional until Terra publishes it.
///
/// A pair `[x, y]` has no direction and stays [`Travel::Unspecified`]. A third
/// number, or `velocity` / `speed` / `linear`, is reverse when negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Travel {
    Forward,
    Reverse,
    Unspecified,
}

/// One sample of `global_route` in local metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RouteSample {
    pub x: f64,
    pub y: f64,
    pub travel: Travel,
}

/// Who is commanding the rover. Provisional until Terra #53 lands.
///
/// Fleet teleop from this station is always constrained. Phone Bluetooth manual
/// is not, and this station does not attach an exemption field to its twists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CommandOrigin {
    #[default]
    Unknown,
    FleetTeleop,
    PhoneBluetooth,
}

/// Version-1 pose quality. Provisional until Terra #53 lands.
///
/// Topic `<prefix>/{id}/pose/quality`. The same object may be nested on
/// autonomy status as `pose_quality`. Unavailable ages are null.
#[derive(Clone, Debug, PartialEq)]
pub struct PoseQuality {
    pub rover_id: u64,
    pub run_id: String,
    pub sequence: u64,
    pub degraded: bool,
    pub reason: String,
    pub pose_age_s: Option<f64>,
    pub imu_age_s: Option<f64>,
}

pub const POSE_QUALITY_SCHEMA: u64 = 1;
/// Heading-only maneuver that cannot run while in-place spins are disabled.
pub const HEADING_HOLD: &str = "heading_requires_spin_or_replan";
pub const HEADING_HOLD_LABEL: &str = "Heading needs a spin or a replan";

/// Status fields the window displays. `assigned` is the study condition Terra enforces.
#[derive(Clone, Debug, PartialEq)]
pub struct AutonomyReport {
    pub requested: String,
    pub effective: Option<String>,
    pub assigned: Option<String>,
    pub supported: Vec<String>,
    pub route: Vec<RouteSample>,
    pub token: Option<String>,
    pub result: Option<String>,
    pub revision: Option<u64>,
    /// `heading_requires_spin_or_replan` when Terra reports that hold.
    pub hold: Option<String>,
    /// Terra's machine-readable explanation of the current decision, such as `sensor_unhealthy`.
    pub reason: Option<String>,
    pub commanded_linear: Option<f64>,
    pub pose_quality: Option<PoseQuality>,
    pub origin: CommandOrigin,
    pub arrival_radius: f64,
    /// Terra's latest `sensor_unhealthy` hold, kept a few seconds after it clears.
    pub sensor_hold: Option<SensorHold>,
    /// Bucket dock, once one has started this run.
    pub dock: Option<crate::DockReport>,
}

/// What tripped Terra's latest `sensor_unhealthy` hold (`autonomy/status.sensor_hold`).
#[derive(Clone, Debug, PartialEq)]
pub struct SensorHold {
    /// Machine-readable cause, such as `stale_vio` or `pose_stale`.
    pub cause: String,
    pub pose_age_s: Option<f64>,
    /// Seconds since the hold.
    pub age_s: f64,
    /// Holds in this run.
    pub count: u64,
}

impl SensorHold {
    /// `stale_vio · pose 0.41 s old`
    pub fn label(&self) -> String {
        match self.pose_age_s {
            Some(age) => format!("{} · pose {age:.2} s old", self.cause),
            None => self.cause.clone(),
        }
    }
}

fn sensor_hold(value: &Value) -> Option<SensorHold> {
    let hold = value.get("sensor_hold")?;
    let cause = hold["cause"].as_str().filter(|cause| !cause.is_empty())?;
    let age_s = hold["age_s"]
        .as_f64()
        .filter(|age| age.is_finite() && *age >= 0.)?;
    Some(SensorHold {
        cause: cause.into(),
        pose_age_s: hold["pose_age_s"].as_f64().filter(|age| age.is_finite()),
        age_s,
        count: hold["count"].as_u64().unwrap_or(0),
    })
}

pub fn decode_autonomy_status(bytes: &[u8]) -> Result<AutonomyReport> {
    if bytes.len() > crate::MAX_STATE_BYTES {
        return Err(invalid("autonomy status too large"));
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| invalid("invalid autonomy JSON"))?;
    decode_autonomy_value(&value)
}

pub fn decode_autonomy_value(value: &Value) -> Result<AutonomyReport> {
    let requested = value["requested_level"]
        .as_str()
        .filter(|level| !level.is_empty())
        .ok_or_else(|| invalid("missing requested level"))?
        .to_string();
    let level = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|level| !level.is_empty())
            .map(str::to_string)
    };
    let supported = value["supported_levels"]
        .as_array()
        .map(|levels| {
            levels
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut route = Vec::new();
    if let Some(points) = value["global_route"].as_array() {
        for point in points {
            if let Some(sample) = route_sample(point) {
                route.push(sample);
            }
        }
    }
    Ok(AutonomyReport {
        requested,
        effective: level("effective_level"),
        assigned: level("assigned_level"),
        supported,
        route,
        token: level("token"),
        result: level("result"),
        revision: value["revision"].as_u64(),
        hold: hold_reason(value),
        reason: level("reason"),
        commanded_linear: commanded_forward(value),
        pose_quality: value
            .get("pose_quality")
            .and_then(|nested| pose_quality_value(nested).ok()),
        origin: command_origin(value),
        arrival_radius: arrival_radius_m(value),
        sensor_hold: sensor_hold(value),
        dock: crate::decode_dock(value),
    })
}

fn route_sample(point: &Value) -> Option<RouteSample> {
    if let Some(pair) = point.as_array() {
        let x = pair.first().and_then(Value::as_f64)?;
        let y = pair.get(1).and_then(Value::as_f64)?;
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let travel = match pair.get(2).and_then(Value::as_f64) {
            Some(speed) if speed.is_finite() && speed < 0. => Travel::Reverse,
            Some(speed) if speed.is_finite() => Travel::Forward,
            _ => Travel::Unspecified,
        };
        return Some(RouteSample { x, y, travel });
    }
    if !point.is_object() {
        return None;
    }
    let x = point.get("x").and_then(Value::as_f64)?;
    let y = point.get("y").and_then(Value::as_f64)?;
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let travel = if point.get("reverse").and_then(Value::as_bool) == Some(true)
        || matches!(
            point.get("direction").and_then(Value::as_str),
            Some("reverse" | "backward")
        ) {
        Travel::Reverse
    } else if let Some(speed) = point
        .get("velocity")
        .or_else(|| point.get("speed"))
        .or_else(|| point.get("linear"))
        .and_then(Value::as_f64)
        .filter(|speed| speed.is_finite())
    {
        if speed < 0. {
            Travel::Reverse
        } else {
            Travel::Forward
        }
    } else {
        Travel::Unspecified
    };
    Some(RouteSample { x, y, travel })
}

fn hold_reason(value: &Value) -> Option<String> {
    for key in ["hold_reason", "reason", "result"] {
        if value.get(key).and_then(Value::as_str) == Some(HEADING_HOLD) {
            return Some(HEADING_HOLD.to_string());
        }
    }
    None
}

fn commanded_forward(value: &Value) -> Option<f64> {
    for key in ["commanded_linear", "linear"] {
        if let Some(speed) = value
            .get(key)
            .and_then(Value::as_f64)
            .filter(|speed| speed.is_finite())
        {
            return Some(speed);
        }
    }
    for nest in ["twist", "command", "output"] {
        if let Some(speed) = value
            .get(nest)
            .and_then(|body| body.get("linear"))
            .and_then(Value::as_f64)
            .filter(|speed| speed.is_finite())
        {
            return Some(speed);
        }
    }
    None
}

pub fn command_origin(value: &Value) -> CommandOrigin {
    let raw = value
        .get("command_origin")
        .or_else(|| value.get("origin"))
        .and_then(Value::as_str);
    match raw {
        Some("fleet_teleop" | "FleetTeleop") => CommandOrigin::FleetTeleop,
        Some("phone_bluetooth" | "bluetooth" | "PhoneBluetooth" | "phone") => {
            CommandOrigin::PhoneBluetooth
        }
        _ => CommandOrigin::Unknown,
    }
}

/// Operator-visible pose text. Phone Bluetooth manual does not show it.
///
/// A reported quality reason is informational for ARGOS teleop. Waypoint sends
/// still use [`crate::goal_blocked`] on the degraded flag.
pub fn pose_warning_text(origin: CommandOrigin, degraded: bool, reason: &str) -> Option<String> {
    if origin == CommandOrigin::PhoneBluetooth || !degraded || reason.is_empty() {
        return None;
    }
    if reason.starts_with("yaw-rate fallback") {
        Some(reason.to_string())
    } else {
        Some(format!(
            "{reason} (informational; manual drive is not blocked)"
        ))
    }
}

pub fn decode_pose_quality(bytes: &[u8]) -> Result<PoseQuality> {
    if bytes.len() > crate::MAX_STATE_BYTES {
        return Err(invalid("pose quality too large"));
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| invalid("invalid pose quality JSON"))?;
    pose_quality_value(&value)
}

pub fn pose_quality_value(value: &Value) -> Result<PoseQuality> {
    if value.get("schema_version").and_then(Value::as_u64) != Some(POSE_QUALITY_SCHEMA) {
        return Err(invalid("pose quality schema"));
    }
    let rover_id = value
        .get("rover_id")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("pose quality rover"))?;
    let run_id = value
        .get("run_id")
        .and_then(Value::as_str)
        .filter(|run| !run.is_empty() && run.len() <= 256)
        .ok_or_else(|| invalid("pose quality run"))?
        .to_string();
    let sequence = value
        .get("sequence")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("pose quality sequence"))?;
    let degraded = value
        .get("degraded")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("pose quality degraded"))?;
    let reason = value
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(PoseQuality {
        rover_id,
        run_id,
        sequence,
        degraded,
        reason,
        pose_age_s: optional_age(value, "pose_age_s")?,
        imu_age_s: optional_age(value, "imu_age_s")?,
    })
}

fn optional_age(value: &Value, key: &str) -> Result<Option<f64>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(age) => age
            .as_f64()
            .filter(|age| age.is_finite() && *age >= 0.)
            .map(Some)
            .ok_or_else(|| invalid("pose quality age")),
    }
}

/// Prefer the dedicated topic, then the copy nested on autonomy status.
pub fn reported_pose_quality(rover: &Value) -> Option<PoseQuality> {
    rover
        .get("pose_quality")
        .and_then(|value| pose_quality_value(value).ok())
        .or_else(|| {
            rover
                .get("authority")
                .and_then(|status| status.get("pose_quality"))
                .and_then(|value| pose_quality_value(value).ok())
        })
}

/// Outdoor arrival radius in metres. Frozen by the caller at drag release.
///
/// Provisional. Terra's reviewed default is 5 m, limited to 2 through 20.
pub fn arrival_radius_m(value: &Value) -> f64 {
    let raw = value
        .get("arrival_radius")
        .and_then(Value::as_f64)
        .or_else(|| {
            value
                .get("navigation")
                .and_then(|body| body.get("arrival_radius"))
                .and_then(Value::as_f64)
        });
    match raw {
        Some(radius) if radius.is_finite() && (2. ..=20.).contains(&radius) => radius,
        _ => 5.,
    }
}

/// Outdoor GPS motion is not implemented on current vehicles.
///
/// The outdoor waypoint spec names the capability `outdoor_gps` on navigation
/// telemetry. A missing field means local goals only.
pub fn outdoor_gps_advertised(value: &Value) -> bool {
    if value.get("outdoor_gps") == Some(&Value::Bool(true)) {
        return true;
    }
    if matches!(
        value.get("source").and_then(Value::as_str),
        Some("outdoor_gps")
    ) || matches!(
        value.get("capability").and_then(Value::as_str),
        Some("outdoor_gps")
    ) {
        return true;
    }
    value
        .get("capability")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.as_str() == Some("outdoor_gps"))
        })
        || value.get("navigation").is_some_and(outdoor_gps_advertised)
        || value.get("authority").is_some_and(outdoor_gps_advertised)
}

/// Geographic goals stay unpublished until the rover advertises `outdoor_gps`.
pub fn geographic_goal_refusal(advertised: bool) -> Option<&'static str> {
    if advertised {
        None
    } else {
        Some("Outdoor GPS goals are unsupported on this rover")
    }
}
