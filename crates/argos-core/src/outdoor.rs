//! Outdoor GPS navigation telemetry and geographic waypoint admission.
//!
//! Version-1 contract shared with Terra's outdoor mission controller.
//! Geographic goal envelopes also carry reviewed run_id, navigation_revision,
//! and arrival_radius_m; coordinates are never converted into the AR frame.
//!
//! Terra publishes version-1 status on `{prefix}/{id}/navigation/outdoor`.
//! The same object may be nested at `outdoor` or `authority.navigation.outdoor`.
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "capability": "outdoor_gps",
//!   "supported": true,
//!   "acknowledged": true,
//!   "source": "outdoor_gps",
//!   "run_id": "run",
//!   "navigation_revision": 1,
//!   "ready": true,
//!   "hold_reason": null,
//!   "gps_age_s": 0.4,
//!   "gps_accuracy_m": 4.0,
//!   "heading_age_s": 0.2,
//!   "heading_accuracy_deg": 8.0,
//!   "heading_deg": 72.5,
//!   "arrival_radius_m": 5.0,
//!   "destination": {"latitude": 38.83, "longitude": -77.31},
//!   "remaining_m": 12.0,
//!   "phase": "planning",
//!   "gps": {"latitude": 38.83, "longitude": -77.30, "accuracy_m": 4.0, "age_s": 0.4},
//!   "token": "ack",
//!   "result": "accepted"
//! }
//! ```
//!
//! A source change is `{prefix}/{id}/navigation/source` :
//! `{"schema_version":1,"source","token","run_id","navigation_revision"}`.
//! Selecting a source requires neutral output and does not start a mission.

use crate::{Result, invalid, valid_token};
use serde_json::{Value, json};
use std::result::Result as StdResult;

pub const OUTDOOR_SCHEMA: u64 = 1;
pub const APP_UPDATE_REQUIRED: &str = "App update required";
pub const OUTDOOR_SOURCE: &str = "outdoor_gps";
pub const LOCAL_SOURCE: &str = "local";
pub const GPS_MAX_AGE_S: f64 = 3.;
pub const GPS_MAX_UNCERTAINTY_M: f64 = 10.;
pub const HEADING_MAX_AGE_S: f64 = 2.;
// Temporary field-test allowance; freshness remains independently enforced.
pub const HEADING_MAX_UNCERTAINTY_DEG: f64 = 35.;
pub const MISSION_MAX_DISTANCE_M: f64 = 500.;
pub const ARRIVAL_RADIUS_DEFAULT_M: f64 = 10.;
pub const ARRIVAL_RADIUS_MIN_M: f64 = 2.;
pub const ARRIVAL_RADIUS_MAX_M: f64 = 20.;
/// How old the outdoor status object itself may be. Provisional.
pub const OUTDOOR_STATUS_MAX_AGE_S: f64 = 3.;

/// Distinct sensor and mission holds reported by Terra.
pub const OUTDOOR_HOLDS: &[(&str, &str)] = &[
    ("gps_stale", "GPS stale"),
    ("gps_uncertain", "GPS uncertain"),
    ("authority_lost", "Authority lost"),
    ("heading_uncertain", "Heading uncertain"),
    ("tracking_lost", "Tracking lost"),
    ("obstacle_map_stale", "Obstacle map stale"),
    ("route_blocked", "Route blocked"),
    ("wheel_feedback_unavailable", "Wheel feedback unavailable"),
    ("hardware_unavailable", "Hardware unavailable"),
    ("paused", "Paused"),
];

/// Version-1 outdoor capability and mission status.
#[derive(Clone, Debug, PartialEq)]
pub struct OutdoorReport {
    pub schema_version: u64,
    pub supported: bool,
    pub acknowledged: bool,
    pub source: String,
    pub run_id: String,
    pub navigation_revision: u64,
    pub ready: bool,
    pub hold_reason: Option<String>,
    pub gps_age_s: Option<f64>,
    pub gps_accuracy_m: Option<f64>,
    pub heading_age_s: Option<f64>,
    pub heading_accuracy_deg: Option<f64>,
    /// Fused true heading, degrees clockwise from north, for the pose of the
    /// same report. Terra omits it while it has no estimate.
    pub heading_deg: Option<f64>,
    pub arrival_radius_m: f64,
    pub destination_latitude: Option<f64>,
    pub destination_longitude: Option<f64>,
    pub remaining_m: Option<f64>,
    pub phase: String,
    pub gps_latitude: Option<f64>,
    pub gps_longitude: Option<f64>,
    pub token: Option<String>,
    pub result: Option<String>,
    /// Age of this status object, seconds. Provisional `age` field.
    pub age_s: Option<f64>,
}

/// Operator-visible GPS fix. Independent of outdoor readiness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawGps {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy_m: Option<f64>,
    pub age_s: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
pub struct OutdoorAdmission<'a> {
    pub report: Option<&'a OutdoorReport>,
    pub latitude: f64,
    pub longitude: f64,
    pub arrival_radius_m: f64,
    /// `Some("waypoint")` is required when the caller is about to publish.
    /// `None` skips the level check so a mode request can still ask for L3.
    pub level: Option<&'a str>,
    pub run_id: &'a str,
    pub navigation_revision: u64,
}

pub fn hold_label(reason: &str) -> Option<&'static str> {
    let reason = reason.trim();
    OUTDOOR_HOLDS
        .iter()
        .find(|(token, label)| *token == reason || *label == reason)
        .map(|(_, label)| *label)
}

pub fn phase_label(phase: &str) -> String {
    match phase {
        "idle" => "Idle".into(),
        "planning" => "Planning".into(),
        "holding" => "Holding".into(),
        "paused" => "Paused".into(),
        "confirming" => "Arrival confirmation".into(),
        "arrived" => "Arrived".into(),
        other => format!("Phase {other} (provisional)"),
    }
}

impl OutdoorReport {
    pub fn decode(value: &Value) -> Result<Self> {
        if value.get("schema_version").and_then(Value::as_u64) != Some(OUTDOOR_SCHEMA) {
            return Err(invalid("outdoor navigation schema"));
        }
        let capability = value.get("capability").and_then(Value::as_str);
        if capability.is_some_and(|name| name != OUTDOOR_SOURCE) {
            return Err(invalid("outdoor navigation capability"));
        }
        let source = value
            .get("source")
            .and_then(Value::as_str)
            .filter(|source| *source == LOCAL_SOURCE || *source == OUTDOOR_SOURCE)
            .ok_or_else(|| invalid("outdoor navigation source"))?
            .to_string();
        let run_id = value
            .get("run_id")
            .and_then(Value::as_str)
            .filter(|run| !run.is_empty() && run.len() <= 256)
            .ok_or_else(|| invalid("outdoor navigation run"))?
            .to_string();
        let navigation_revision = value
            .get("navigation_revision")
            .and_then(revision_u64)
            .ok_or_else(|| invalid("outdoor navigation revision"))?;
        let (destination_latitude, destination_longitude) = match value.get("destination") {
            None | Some(Value::Null) => (None, None),
            Some(body) => (
                finite_coord(body, "latitude", 85.),
                finite_coord(body, "longitude", 180.),
            ),
        };
        let (gps_latitude, gps_longitude) = match value.get("gps") {
            None | Some(Value::Null) => (None, None),
            Some(body) => (
                finite_coord(body, "latitude", 90.),
                finite_coord(body, "longitude", 180.),
            ),
        };
        Ok(Self {
            schema_version: OUTDOOR_SCHEMA,
            supported: value.get("supported").and_then(Value::as_bool) != Some(false)
                && capability == Some(OUTDOOR_SOURCE),
            acknowledged: value.get("acknowledged").and_then(Value::as_bool) == Some(true),
            source,
            run_id,
            navigation_revision,
            ready: value.get("ready").and_then(Value::as_bool) == Some(true),
            hold_reason: value
                .get("hold_reason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.is_empty())
                .map(str::to_string),
            gps_age_s: finite_nonneg(value, "gps_age_s"),
            gps_accuracy_m: finite_nonneg(value, "gps_accuracy_m"),
            heading_age_s: finite_nonneg(value, "heading_age_s"),
            heading_accuracy_deg: finite_nonneg(value, "heading_accuracy_deg"),
            heading_deg: value
                .get("heading_deg")
                .and_then(Value::as_f64)
                .filter(|degrees| degrees.is_finite())
                .map(|degrees| degrees.rem_euclid(360.)),
            arrival_radius_m: finite_nonneg(value, "arrival_radius_m")
                .unwrap_or(ARRIVAL_RADIUS_DEFAULT_M),
            destination_latitude,
            destination_longitude,
            remaining_m: finite_nonneg(value, "remaining_m"),
            phase: value
                .get("phase")
                .and_then(Value::as_str)
                .filter(|phase| !phase.is_empty())
                .unwrap_or("idle")
                .to_string(),
            gps_latitude,
            gps_longitude,
            token: value
                .get("token")
                .and_then(Value::as_str)
                .filter(|token| !token.is_empty())
                .map(str::to_string),
            result: value
                .get("result")
                .and_then(Value::as_str)
                .filter(|result| !result.is_empty())
                .map(str::to_string),
            age_s: finite_nonneg(value, "age"),
        })
    }

    pub fn acknowledged_outdoor(&self) -> bool {
        self.supported && self.acknowledged && self.source == OUTDOOR_SOURCE
    }

    pub fn hold_label(&self) -> Option<&'static str> {
        self.hold_reason.as_deref().and_then(hold_label)
    }

    /// The hold an operator needs to see. Terra reports outdoor readiness
    /// even under the local source, where GPS and heading gate nothing:
    /// L1–L4 run on ARKit pose and the occupancy map alone.
    pub fn operator_hold_label(&self) -> Option<&'static str> {
        if self.acknowledged_outdoor() || self.mission_active() {
            self.hold_label()
        } else {
            None
        }
    }

    pub fn mission_active(&self) -> bool {
        matches!(
            self.phase.as_str(),
            "planning" | "holding" | "paused" | "confirming" | "active"
        )
    }
}

pub fn decode_outdoor(bytes: &[u8]) -> Result<OutdoorReport> {
    if bytes.len() > crate::MAX_STATE_BYTES {
        return Err(invalid("outdoor navigation too large"));
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| invalid("invalid outdoor navigation JSON"))?;
    OutdoorReport::decode(&value)
}

/// Prefer the dedicated topic, then a nested copy on the rover's operator object.
pub fn reported_outdoor(rover: &Value) -> Option<OutdoorReport> {
    let decode = |value: &Value| OutdoorReport::decode(value).ok();
    rover
        .get("outdoor")
        .and_then(decode)
        .or_else(|| {
            rover
                .get("authority")
                .and_then(|status| status.get("outdoor"))
                .and_then(decode)
        })
        .or_else(|| {
            rover
                .get("authority")
                .and_then(|status| status.get("navigation"))
                .and_then(|navigation| navigation.get("outdoor"))
                .and_then(decode)
        })
}

/// Finite WGS84 observation. Age does not hide the fix; admission checks age.
pub fn decode_raw_gps(value: &Value) -> Option<RawGps> {
    let latitude = value
        .get("gpsLatitude")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && value.abs() <= 90.)?;
    let longitude = value
        .get("gpsLongitude")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && value.abs() <= 180.)?;
    let accuracy_m = value
        .get("gpsAccuracy")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.);
    let received = value
        .get("age")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.);
    let fix = value
        .get("gpsFixAge")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.);
    let age_s = match (received, fix) {
        (Some(received), Some(fix)) => Some(received + fix),
        (Some(received), None) => Some(received),
        (None, Some(fix)) => Some(fix),
        (None, None) => None,
    };
    Some(RawGps {
        latitude,
        longitude,
        accuracy_m,
        age_s,
    })
}

pub fn app_update_required(report: Option<&OutdoorReport>) -> bool {
    report.is_none_or(|report| !report.supported || report.schema_version != OUTDOOR_SCHEMA)
}

/// Geographic admission. The WGS84 destination is not rewritten here.
pub fn admit_outdoor_goal(gate: OutdoorAdmission<'_>) -> StdResult<(), &'static str> {
    let Some(report) = gate.report else {
        return Err(APP_UPDATE_REQUIRED);
    };
    if app_update_required(Some(report)) {
        return Err(APP_UPDATE_REQUIRED);
    }
    if !report.acknowledged_outdoor() {
        return Err("Outdoor GPS is not acknowledged");
    }
    if report.run_id != gate.run_id {
        return Err("Rover run changed; review a new waypoint");
    }
    if report.navigation_revision != gate.navigation_revision {
        return Err("Navigation revision changed; review a new waypoint");
    }
    if report
        .age_s
        .is_none_or(|age| age > OUTDOOR_STATUS_MAX_AGE_S)
    {
        return Err("Outdoor navigation telemetry is stale");
    }
    if let Some(label) = report.hold_label() {
        return Err(label);
    }
    if report.hold_reason.is_some() {
        return Err("Outdoor navigation is holding");
    }
    if !report.ready {
        return Err("Outdoor navigation is not ready");
    }
    if !(ARRIVAL_RADIUS_MIN_M..=ARRIVAL_RADIUS_MAX_M).contains(&gate.arrival_radius_m) {
        return Err("Arrival radius must be from 2 through 20 m");
    }
    let (Some(latitude), Some(longitude)) = (report.gps_latitude, report.gps_longitude) else {
        return Err("GPS stale");
    };
    if !latitude.is_finite() || !longitude.is_finite() {
        return Err("GPS stale");
    }
    if report.gps_age_s.is_none_or(|age| age > GPS_MAX_AGE_S) {
        return Err("GPS stale");
    }
    let accuracy = report.gps_accuracy_m.ok_or("GPS stale")?;
    if accuracy > GPS_MAX_UNCERTAINTY_M {
        return Err("GPS uncertainty exceeds 10 m");
    }
    if accuracy > gate.arrival_radius_m {
        return Err("GPS uncertainty is larger than the arrival radius");
    }
    if report
        .heading_age_s
        .is_none_or(|age| age > HEADING_MAX_AGE_S)
        || report
            .heading_accuracy_deg
            .is_none_or(|uncertainty| uncertainty > HEADING_MAX_UNCERTAINTY_DEG)
    {
        return Err("Heading uncertain");
    }
    if !gate.latitude.is_finite()
        || !gate.longitude.is_finite()
        || !(-85. ..=85.).contains(&gate.latitude)
        || !(-180. ..=180.).contains(&gate.longitude)
    {
        return Err("geographic coordinates outside Terra bounds");
    }
    let distance = geodesic_metres(latitude, longitude, gate.latitude, gate.longitude);
    if !distance.is_finite() || distance > MISSION_MAX_DISTANCE_M {
        return Err("Mission distance exceeds 500 m");
    }
    if let Some(level) = gate.level
        && level != "waypoint"
    {
        return Err("Outdoor GPS missions require L3 Waypoint");
    }
    Ok(())
}

pub fn freeze_arrival_radius(requested: Option<f64>, reported: f64) -> StdResult<f64, String> {
    let radius = requested.unwrap_or(reported);
    if (ARRIVAL_RADIUS_MIN_M..=ARRIVAL_RADIUS_MAX_M).contains(&radius) {
        Ok(radius)
    } else {
        Err("Arrival radius must be from 2 through 20 m".into())
    }
}

pub fn source_change_refusal(
    ready_and_neutral: bool,
    goal_active: bool,
    mission_active: bool,
    source: &str,
    report: Option<&OutdoorReport>,
) -> Option<&'static str> {
    if source != LOCAL_SOURCE && source != OUTDOOR_SOURCE {
        return Some("Unsupported navigation source");
    }
    if !ready_and_neutral {
        return Some("Navigation source changes require ready hardware and neutral output");
    }
    if goal_active || mission_active {
        return Some("Navigation source changes require no active mission");
    }
    if source == OUTDOOR_SOURCE && app_update_required(report) {
        return Some(APP_UPDATE_REQUIRED);
    }
    None
}

pub fn encode_source_request(
    source: &str,
    token: &str,
    run_id: &str,
    navigation_revision: u64,
) -> Result<Vec<u8>> {
    if source != LOCAL_SOURCE && source != OUTDOOR_SOURCE {
        return Err(invalid("unsupported navigation source"));
    }
    if !valid_token(token) {
        return Err(invalid("invalid navigation token"));
    }
    if run_id.is_empty() || run_id.len() > 256 {
        return Err(invalid("invalid navigation run"));
    }
    let bytes = serde_json::to_vec(&json!({
        "schema_version": OUTDOOR_SCHEMA,
        "source": source,
        "arrival_radius_m": ARRIVAL_RADIUS_DEFAULT_M,
        "token": token,
        "run_id": run_id,
        "navigation_revision": navigation_revision,
    }))
    .map_err(|error| crate::Error(error.to_string()))?;
    if bytes.len() > 2048 {
        return Err(invalid("navigation source exceeds 2048 bytes"));
    }
    Ok(bytes)
}

pub fn geodesic_metres(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let radius = 6_378_137.0;
    let (lat1, lon1, lat2, lon2) = (
        lat1.to_radians(),
        lon1.to_radians(),
        lat2.to_radians(),
        lon2.to_radians(),
    );
    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;
    let a = (dlat * 0.5).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon * 0.5).sin().powi(2);
    2. * radius * a.sqrt().clamp(0., 1.).asin()
}

fn revision_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn finite_nonneg(value: &Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number >= 0.)
}

fn finite_coord(value: &Value, key: &str, limit: f64) -> Option<f64> {
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && number.abs() <= limit)
}

/// A resume is operator intent, never an automatic consequence of recovered sensors.
/// The vehicle rechecks pose, map, feedback and hardware readiness at receipt.
pub fn outdoor_resume_refusal(
    report: &OutdoorReport,
    run: &str,
    revision: u64,
) -> Option<&'static str> {
    if app_update_required(Some(report)) || !report.acknowledged_outdoor() {
        return Some("Outdoor GPS is not acknowledged");
    }
    if report.run_id != run || report.navigation_revision != revision {
        return Some("Navigation identity changed");
    }
    if report.phase != "paused" {
        return Some("No paused outdoor mission");
    }
    if report.age_s.is_none_or(|a| a > OUTDOOR_STATUS_MAX_AGE_S) {
        return Some("Outdoor status stale");
    }
    if report.gps_age_s.is_none_or(|a| a > GPS_MAX_AGE_S)
        || report
            .gps_accuracy_m
            .is_none_or(|a| a > GPS_MAX_UNCERTAINTY_M || a > report.arrival_radius_m)
    {
        return Some("GPS stale or uncertain");
    }
    if report.heading_age_s.is_none_or(|a| a > HEADING_MAX_AGE_S)
        || report
            .heading_accuracy_deg
            .is_none_or(|a| a > HEADING_MAX_UNCERTAINTY_DEG)
    {
        return Some("Heading uncertain");
    }
    None
}
