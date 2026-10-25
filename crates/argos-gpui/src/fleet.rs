//! Background Zenoh subscription and the commands the selected rover may receive.
//!
//! Connect and snapshot polling run on a thread named `argos-fleet`. Publishing
//! stays on that client and is idle until the window selects a rover. Only the
//! selected rover's occupancy grid, point cloud, and depth are subscribed; the
//! rest of the fleet sends pose and status only.

use argos_core::{
    APP_UPDATE_REQUIRED, CellClass, CommandOrigin, DegradeLimits, ExplorationReport, GoalRequest,
    GoalState, HardwareObservation, LocalFrame, LocalMap, NavigationEdit, OccupancyGrid,
    OutdoorAdmission, OutdoorReport, Pose, PoseWatch, RawGps, RouteSample, Snapshot, TeleopLimits,
    Topics, admit_outdoor_goal, classify_cell, decode_autonomy_value, decode_exploration,
    decode_raw_gps, drive_block, encode_navigation_settings, encode_source_request,
    fallback_degrade_reason, goal_blocked, local_map, map_heading_degrees, reported_outdoor,
    reported_pose_quality, source_change_refusal,
};
use argos_zenoh::Client;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// GMU Fairfax. Used only when the start-location env vars are unset.
pub const DEFAULT_START_LATITUDE: f64 = 38.8297;
pub const DEFAULT_START_LONGITUDE: f64 = -77.3075;
pub const DEFAULT_ENDPOINT: &str = "tls/argos.34.135.204.210.sslip.io:7448";
pub const DEFAULT_PREFIX: &str = "terra/phone";
pub const DEFAULT_USER: &str = "terra";
pub const PASSWORD_NOTICE: &str = "Set ARGOS_ZENOH_PASSWORD in .env";
pub const MAPS_API_KEY_ENV: &str = "GOOGLE_MAPS_API_KEY";
pub const MAPS_KEY_NOTICE: &str = "Set GOOGLE_MAPS_API_KEY in .env";
const POSE_FRESH_SECONDS: f64 = 2.5;
const SETTING_KEYS: [&str; 21] = [
    "ARGOS_ZENOH_ENDPOINT",
    "ARGOS_ZENOH_PREFIX",
    "ARGOS_ZENOH_USER",
    "ARGOS_ZENOH_PASSWORD",
    "ARGOS_MAP_START_LAT",
    "ARGOS_MAP_START_LON",
    "ARGOS_MAP_ORIGIN_LAT",
    "ARGOS_MAP_ORIGIN_LON",
    "ARGOS_DRIVE_LINEAR",
    "ARGOS_DRIVE_YAW_RATE",
    "ARGOS_DRIVE_YAW_ACCEL",
    "ARGOS_DRIVE_YAW_DECEL",
    "ARGOS_DRIVE_MIN_RADIUS",
    "ARGOS_DRIVE_ALLOW_SPIN",
    "ARGOS_DEGRADED_YAW_RATE",
    "ARGOS_DEGRADED_SETTLE",
    MAPS_API_KEY_ENV,
    "ARGOS_FPV_STUN",
    "ARGOS_FPV_TURN_URL",
    "ARGOS_FPV_TURN_USERNAME",
    "ARGOS_FPV_TURN_CREDENTIAL",
];

#[derive(Clone)]
pub struct DashboardConfig {
    pub endpoint: String,
    pub prefix: String,
    pub credentials: Option<(String, String)>,
    /// Sidebar text when the hosted router needs a password that was not provided.
    /// The window still opens, and no connection is attempted.
    pub connect_notice: Option<String>,
    pub start_latitude: f64,
    pub start_longitude: f64,
    pub origin: LocalFrame,
    /// Defense in depth; Terra enforces the real limits.
    pub teleop: TeleopLimits,
    pub degrade: DegradeLimits,
    /// Tiles key resolved from the environment, then `.env`. Never logged.
    pub maps_api_key: Option<String>,
    /// Set when tiles need a key and none was loaded. Absent in `MAPS_DEMO=1`.
    pub maps_notice: Option<String>,
    /// ICE servers for the receive-only camera. Credentials stay in this struct.
    pub fpv: crate::fpv::FpvConfig,
}
impl DashboardConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::from_env_mode(crate::LaunchMode::Real)
    }
    pub fn from_env_mode(mode: crate::LaunchMode) -> Result<Self, String> {
        let mut process = BTreeMap::new();
        for key in SETTING_KEYS {
            if std::env::var_os(key).is_some() {
                process.insert(key.to_string(), std::env::var(key).unwrap_or_default());
            }
        }
        mode.apply(&mut process);
        let files: Vec<_> = dotenv_paths()
            .iter()
            .map(|path| read_dotenv(path))
            .collect();
        let file = merge_dotenv(&process, &files);
        let mut settings = file.clone();
        for (key, value) in &process {
            settings.insert(key.clone(), value.clone());
        }
        let endpoint = process
            .get("ARGOS_ZENOH_ENDPOINT")
            .cloned()
            .or_else(|| file.get("ARGOS_ZENOH_ENDPOINT").cloned());
        let prefix = process
            .get("ARGOS_ZENOH_PREFIX")
            .cloned()
            .or_else(|| file.get("ARGOS_ZENOH_PREFIX").cloned());
        let user = process
            .get("ARGOS_ZENOH_USER")
            .cloned()
            .or_else(|| file.get("ARGOS_ZENOH_USER").cloned());
        let password = process
            .get("ARGOS_ZENOH_PASSWORD")
            .cloned()
            .or_else(|| file.get("ARGOS_ZENOH_PASSWORD").cloned());
        let start_lat = process
            .get("ARGOS_MAP_START_LAT")
            .cloned()
            .or_else(|| file.get("ARGOS_MAP_START_LAT").cloned());
        let start_lon = process
            .get("ARGOS_MAP_START_LON")
            .cloned()
            .or_else(|| file.get("ARGOS_MAP_START_LON").cloned());
        let origin_lat = process
            .get("ARGOS_MAP_ORIGIN_LAT")
            .cloned()
            .or_else(|| file.get("ARGOS_MAP_ORIGIN_LAT").cloned());
        let origin_lon = process
            .get("ARGOS_MAP_ORIGIN_LON")
            .cloned()
            .or_else(|| file.get("ARGOS_MAP_ORIGIN_LON").cloned());
        let mut config = Self::parse(
            endpoint.as_deref(),
            prefix.as_deref(),
            user.as_deref(),
            password.as_deref(),
            (start_lat.as_deref(), start_lon.as_deref()),
            (origin_lat.as_deref(), origin_lon.as_deref()),
        )?;
        apply_drive_settings(&mut config, &settings)?;
        let maps_api_key = maps_api_key(&process, &file);
        let maps_demo = std::env::var("MAPS_DEMO").ok().as_deref() == Some("1");
        config.maps_notice = if maps_api_key.is_none() && !maps_demo {
            Some(MAPS_KEY_NOTICE.to_string())
        } else {
            None
        };
        config.maps_api_key = maps_api_key;
        config.fpv = crate::fpv::FpvConfig::from_values(&settings)?;
        Ok(config)
    }
    pub fn parse(
        endpoint: Option<&str>,
        prefix: Option<&str>,
        user: Option<&str>,
        password: Option<&str>,
        start: (Option<&str>, Option<&str>),
        origin: (Option<&str>, Option<&str>),
    ) -> Result<Self, String> {
        let endpoint = match endpoint.map(str::trim).filter(|s| !s.is_empty()) {
            Some(endpoint) => endpoint.to_string(),
            None => DEFAULT_ENDPOINT.into(),
        };
        if endpoint.len() > 512 || !(endpoint.starts_with("tcp/") || endpoint.starts_with("tls/")) {
            return Err("ARGOS_ZENOH_ENDPOINT must be tcp/host:port or tls/host:port".into());
        }
        let prefix = prefix.map(str::trim).filter(|s| !s.is_empty());
        let prefix = Topics::new(prefix.unwrap_or(DEFAULT_PREFIX)).map_err(|e| e.to_string())?;
        let user = match user.map(str::trim) {
            None => Some(DEFAULT_USER),
            Some("") => None,
            Some(user) => Some(user),
        };
        let password = password.map(str::trim).filter(|s| !s.is_empty());
        let (credentials, connect_notice) =
            if endpoint.starts_with("tls/") && password.is_none() && user.is_some() {
                (None, Some(PASSWORD_NOTICE.to_string()))
            } else {
                let credentials = match (user, password) {
                    (None, None) => None,
                    (Some(user), Some(password)) => Some((user.to_string(), password.to_string())),
                    _ => {
                        return Err(
                        "ARGOS_ZENOH_USER and ARGOS_ZENOH_PASSWORD must both be set, or neither"
                            .into(),
                    );
                    }
                };
                (credentials, None)
            };
        let start_latitude = coordinate("ARGOS_MAP_START_LAT", start.0, DEFAULT_START_LATITUDE)?;
        let start_longitude = coordinate("ARGOS_MAP_START_LON", start.1, DEFAULT_START_LONGITUDE)?;
        if !(-90. ..=90.).contains(&start_latitude) || !(-180. ..=180.).contains(&start_longitude) {
            return Err("ARGOS_MAP_START_LAT and ARGOS_MAP_START_LON must be WGS84 degrees".into());
        }
        let origin_latitude = optional_coordinate("ARGOS_MAP_ORIGIN_LAT", origin.0)?;
        let origin_longitude = optional_coordinate("ARGOS_MAP_ORIGIN_LON", origin.1)?;
        let (origin_latitude, origin_longitude) = match (origin_latitude, origin_longitude) {
            (None, None) => (start_latitude, start_longitude),
            (Some(latitude), Some(longitude)) => (latitude, longitude),
            _ => {
                return Err(
                    "ARGOS_MAP_ORIGIN_LAT and ARGOS_MAP_ORIGIN_LON must both be set, or neither"
                        .into(),
                );
            }
        };
        let origin = LocalFrame::origin(origin_latitude, origin_longitude).map_err(|_| {
            "local-metre origin is outside the projection range; set ARGOS_MAP_ORIGIN_LAT and ARGOS_MAP_ORIGIN_LON"
        })?;
        Ok(Self {
            endpoint,
            prefix: prefix.prefix,
            credentials,
            connect_notice,
            start_latitude,
            start_longitude,
            origin,
            teleop: TeleopLimits::default(),
            degrade: DegradeLimits::default(),
            maps_api_key: None,
            maps_notice: None,
            fpv: crate::fpv::FpvConfig::default(),
        })
    }
}
/// `GOOGLE_MAPS_API_KEY` from the process, otherwise from `.env`. Empty is missing.
/// A process value wins even when it is blank, so the file cannot replace it.
pub fn maps_api_key(
    process: &BTreeMap<String, String>,
    file: &BTreeMap<String, String>,
) -> Option<String> {
    let raw = if process.contains_key(MAPS_API_KEY_ENV) {
        process.get(MAPS_API_KEY_ENV)
    } else {
        file.get(MAPS_API_KEY_ENV)
    }?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
pub fn apply_drive_settings(
    config: &mut DashboardConfig,
    values: &BTreeMap<String, String>,
) -> Result<(), String> {
    let mut teleop = config.teleop;
    let mut degrade = config.degrade;
    if let Some(value) = number(values, "ARGOS_DRIVE_LINEAR")? {
        teleop.linear = value;
    }
    if let Some(value) = number(values, "ARGOS_DRIVE_YAW_RATE")? {
        teleop.yaw_rate = value;
    }
    if let Some(value) = number(values, "ARGOS_DRIVE_YAW_ACCEL")? {
        teleop.yaw_accel = value;
    }
    if let Some(value) = number(values, "ARGOS_DRIVE_YAW_DECEL")? {
        teleop.yaw_decel = value;
    }
    if let Some(value) = number(values, "ARGOS_DRIVE_MIN_RADIUS")? {
        teleop.min_turn_radius = value;
    }
    if let Some(value) = values.get("ARGOS_DRIVE_ALLOW_SPIN") {
        teleop.allow_spin = matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        );
    }
    if let Some(value) = number(values, "ARGOS_DEGRADED_YAW_RATE")? {
        degrade.yaw_rate = value;
    }
    if let Some(value) = number(values, "ARGOS_DEGRADED_SETTLE")? {
        degrade.settle = value;
    }
    config.teleop = teleop.validate().map_err(|error| error.to_string())?;
    if !degrade.yaw_rate.is_finite()
        || !degrade.settle.is_finite()
        || degrade.yaw_rate <= 0.
        || degrade.settle < 0.
    {
        return Err("degraded-pose limits must be finite and the yaw rate positive".into());
    }
    config.degrade = degrade;
    Ok(())
}
fn number(values: &BTreeMap<String, String>, key: &str) -> Result<Option<f64>, String> {
    let Some(raw) = values
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let value: f64 = raw
        .parse()
        .map_err(|_| format!("{key} must be a finite number"))?;
    if !value.is_finite() {
        return Err(format!("{key} must be a finite number"));
    }
    Ok(Some(value))
}
/// Lowest priority first: the installed app's folders, then the workspace
/// root for `cargo run`, then the current directory.
fn dotenv_paths() -> Vec<PathBuf> {
    let exe = std::env::current_exe().ok();
    let mut paths: Vec<PathBuf> =
        crate::install::install_env_dirs(exe.as_deref(), crate::install::user_config_dir())
            .into_iter()
            .map(|dir| dir.join(".env"))
            .collect();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(root) = manifest.parent().and_then(Path::parent) {
        let path = root.join(".env");
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let path = cwd.join(".env");
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    }
    paths
}
/// Keys from a dotenv file. A missing or unreadable file is empty.
pub fn read_dotenv(path: &Path) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    let Ok(entries) = dotenvy::from_path_iter(path) else {
        return values;
    };
    for entry in entries {
        let Ok((key, value)) = entry else {
            continue;
        };
        values.insert(key, value);
    }
    values
}
/// File maps are lowest priority first. Keys already in `process` are left out.
pub fn merge_dotenv(
    process: &BTreeMap<String, String>,
    files_low_to_high: &[BTreeMap<String, String>],
) -> BTreeMap<String, String> {
    let mut merged = BTreeMap::new();
    for file in files_low_to_high {
        for (key, value) in file {
            if !process.contains_key(key) {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
    merged
}
fn coordinate(name: &str, raw: Option<&str>, default: f64) -> Result<f64, String> {
    Ok(optional_coordinate(name, raw)?.unwrap_or(default))
}
fn optional_coordinate(name: &str, raw: Option<&str>) -> Result<Option<f64>, String> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let value: f64 = raw
        .parse()
        .map_err(|_| format!("{name} must be a finite number"))?;
    if !value.is_finite() {
        return Err(format!("{name} must be a finite number"));
    }
    Ok(Some(value))
}

#[derive(Clone, Debug, PartialEq)]
pub struct GoalMark {
    pub x: f64,
    pub y: f64,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub state: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RoverMark {
    pub id: u64,
    pub latitude: f64,
    pub longitude: f64,
    /// Geographic transform only when alignment is actually supplied (or no phone report exists).
    pub map_frame: Option<LocalFrame>,
    /// Frame for the rover's local map, route, and local goals on the main map.
    /// The localization frame when there is one; otherwise pinned to the GPS
    /// fix and Terra's fused heading. Map clicks convert through it, so a
    /// local goal lands on the grid cell that was clicked.
    pub overlay_frame: Option<LocalFrame>,
    pub heading: f64,
    pub local_x: f64,
    pub local_y: f64,
    pub yaw: f64,
    pub sequence: u64,
    /// `online`, `pose stale`, or `offline`.
    pub status: String,
    pub occupancy: String,
    pub grid: Option<LocalMap>,
    pub requested_level: Option<String>,
    pub effective_level: Option<String>,
    pub recommended_level: Option<String>,
    pub route: Vec<RouteSample>,
    pub goal: Option<GoalMark>,
    pub degraded: bool,
    pub degrade_reason: String,
    /// True when a pose/quality report, not the yaw-rate fallback, set `degraded`.
    pub quality_reported: bool,
    pub reversing: bool,
    pub hold: Option<String>,
    pub command_origin: CommandOrigin,
    /// Metres. Frozen from the reviewed outdoor report, otherwise the 10 m default.
    pub arrival_radius: f64,
    /// True when outdoor GPS is the acknowledged navigation source.
    pub outdoor_gps: bool,
    pub hardware: HardwareObservation,
    pub authority_revision: u64,
    pub safety: String,
    pub run_id: String,
    pub navigation_revision: u64,
    /// Terra's planner settings status (`authority.navigation`); `Null` until reported.
    pub navigation_settings: Value,
    pub outdoor: Option<OutdoorReport>,
    pub raw_gps: Option<RawGps>,
    pub pose_available: bool,
    pub gps_display_freshness_seconds: f64,
    pub gps_source_fresh: bool,
    pub app_update_required: bool,
    pub mission_active: bool,
    pub goal_active: bool,
    /// Terra's L4 run status while exploring or after a run ends.
    pub exploration: Option<ExplorationReport>,
}
impl RoverMark {
    /// L0 uses direct operator effort; a missing camera pose is only a map warning.
    pub fn control_pose_ready(&self) -> bool {
        self.pose_available
            && (self.status == "online"
                || (self.status == "pose stale"
                    && self.requested_level.as_deref() == Some("teleop")))
    }
}

/// Fleet identity is independent of a drawable geographic marker.
#[derive(Clone, Debug)]
pub struct RoverPresence {
    id: u64,
    membership: String,
    pose_available: bool,
    observation_age: Option<f64>,
    position_reason: String,
    source_reason: String,
}
impl RoverPresence {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn membership(&self) -> &str {
        &self.membership
    }
    pub fn pose_available(&self) -> bool {
        self.pose_available
    }
    pub fn observation_age(&self) -> Option<f64> {
        self.observation_age
    }
    pub fn position_reason(&self) -> &str {
        &self.position_reason
    }
}
pub fn fleet_presence(snapshot: &Snapshot, marks: &[RoverMark]) -> Vec<RoverPresence> {
    // Defensively deduplicate wire membership; retired entries remain explicitly absent.
    snapshot
        .rovers
        .iter()
        .map(|r| (r.id, r))
        .collect::<BTreeMap<_, _>>()
        .values()
        .map(|r| RoverPresence {
            id: r.id,
            membership: r.membership.clone(),
            pose_available: r.pose.is_some(),
            observation_age: r.pose_age.or(snapshot.fleet_age),
            source_reason: String::new(),
            position_reason: marks
                .iter()
                .find(|m| m.id == r.id)
                .map(position_label)
                .unwrap_or_else(|| "Position unavailable".into()),
        })
        .collect()
}
pub fn position_label(mark: &RoverMark) -> String {
    if let Some(gps) = mark.raw_gps {
        let fresh = gps
            .age_s
            .is_some_and(|age| age <= mark.gps_display_freshness_seconds);
        format!(
            "{}{}",
            if fresh && mark.gps_source_fresh {
                "GPS observation"
            } else {
                "Last known GPS"
            },
            gps.age_s
                .map(|age| format!(" · {age:.1} s old"))
                .unwrap_or_else(|| " · age unknown".into())
        )
    } else if mark.status == "online" {
        "Local map position".into()
    } else {
        "Last known local position".into()
    }
}
fn scope_localization(mut report: Value, current_run: Option<&str>) -> Value {
    if let Some(run) = current_run {
        if report["run_id"]
            .as_str()
            .is_some_and(|source| source != run)
        {
            // Keep explicit local identity so a pending report cannot invoke the default origin.
            report = serde_json::json!({"version":1,"mode":"local","frameID":report["frameID"],
                "reason":"Position unavailable · waiting for localization from the current run"});
        }
        report["run_id"] = Value::String(run.into());
    }
    report
}

/// Retain display observations across empty reports/reconnects, never across frames/runs.
#[derive(Default)]
struct LocalizationHistory {
    values: BTreeMap<String, (Value, f64)>,
    membership: BTreeMap<u64, String>,
    retired_at: BTreeMap<u64, f64>,
}
impl LocalizationHistory {
    fn observe_membership(&mut self, rows: &[(u64, &str)], now: f64) {
        let ids: BTreeSet<_> = rows.iter().map(|r| r.0).collect();
        self.membership.retain(|id, _| ids.contains(id));
        self.retired_at.retain(|id, _| ids.contains(id));
        for (id, state) in rows {
            if *state == "absent" && self.membership.get(id).is_none_or(|old| old != "absent") {
                self.retired_at.insert(*id, now);
                self.values.remove(&id.to_string());
            }
            self.membership.insert(*id, (*state).into());
            if *state != "absent" && self.retired_at.contains_key(id) {
                self.values.entry(id.to_string()).or_insert_with(|| (serde_json::json!({"version":1,"mode":"local",
                    "reason":"Position unavailable · waiting for a new observation after retirement"}), now));
            }
        }
    }
    fn retain(&mut self, ids: &BTreeSet<u64>) {
        self.values
            .retain(|id, _| id.parse::<u64>().is_ok_and(|id| ids.contains(&id)));
    }
    fn scope_runs(&mut self, operator: &Value) {
        for (id, (report, _)) in &mut self.values {
            *report =
                scope_localization(report.clone(), operator[id]["authority"]["run_id"].as_str());
        }
    }
    fn update(&mut self, incoming: &Value, now: f64) -> Value {
        if let Some(reports) = incoming.as_object() {
            for (id, report) in reports {
                let mut report = report.clone();
                if let Ok(rover) = id.parse::<u64>()
                    && let Some(cutoff) = self.retired_at.get(&rover)
                {
                    let source_time = now - report["age"].as_f64().unwrap_or(0.);
                    if self
                        .membership
                        .get(&rover)
                        .is_some_and(|state| state == "absent")
                        || source_time <= *cutoff
                    {
                        report = serde_json::json!({"version":1,"mode":"local","frameID":report["frameID"],"run_id":report["run_id"],
                            "reason":"Position unavailable · waiting for a new observation after retirement"});
                    } else if decode_raw_gps(&report)
                        .is_some_and(|gps| now - gps.age_s.unwrap_or(f64::INFINITY) <= *cutoff)
                        && let Some(o) = report.as_object_mut()
                    {
                        o.remove("gpsLatitude");
                        o.remove("gpsLongitude");
                        o.insert("gpsFresh".into(), Value::Bool(false));
                    }
                }
                if let Some((previous, at)) = self.values.get(id) {
                    let same_frame = report["frameID"] == previous["frameID"]
                        && report["run_id"] == previous["run_id"];
                    if same_frame
                        && decode_raw_gps(&report).is_none()
                        && let Some(gps) = decode_raw_gps(previous)
                    {
                        report["gpsLatitude"] = serde_json::json!(gps.latitude);
                        report["gpsLongitude"] = serde_json::json!(gps.longitude);
                        report["gpsAccuracy"] = serde_json::json!(gps.accuracy_m);
                        report["gpsObservedAt"] = previous["gpsObservedAt"].clone();
                        report["gpsFresh"] = serde_json::json!(false);
                        let age = gps.age_s.map(|age| {
                            (age + (now - at).max(0.) - report["age"].as_f64().unwrap_or(0.))
                                .max(0.)
                        });
                        report["gpsFixAge"] = serde_json::json!(age);
                    }
                }
                self.values.insert(id.clone(), (report, now));
            }
        }
        Value::Object(
            self.values
                .iter()
                .map(|(id, (value, at))| {
                    let mut value = value.clone();
                    value["age"] =
                        serde_json::json!(value["age"].as_f64().unwrap_or(0.) + (now - at).max(0.));
                    (id.clone(), value)
                })
                .collect(),
        )
    }
}

#[derive(Clone, Debug)]
pub struct FleetPicture {
    pub link: String,
    pub detail: String,
    pub command: String,
    pub marks: Vec<RoverMark>,
    pub presence: Vec<RoverPresence>,
    pub observed_at: std::time::Instant,
}
impl Default for FleetPicture {
    fn default() -> Self {
        Self {
            link: "connecting".into(),
            detail: "Connecting".into(),
            command: String::new(),
            marks: Vec::new(),
            presence: Vec::new(),
            observed_at: std::time::Instant::now(),
        }
    }
}

pub fn fleet_marks(
    snapshot: &Snapshot,
    localizations: &Value,
    operator: &Value,
    mut occupancy: impl FnMut(u64) -> Option<(OccupancyGrid, f64)>,
    origin: LocalFrame,
) -> Vec<RoverMark> {
    snapshot
        .rovers
        .iter()
        .filter(|rover| rover.membership != "absent")
        .filter_map(|rover| {
            let pose = rover.pose.as_ref()?;
            let key = rover.id.to_string();
            let localization = localizations.get(&key);
            let raw_gps = decode_raw_gps(localization.unwrap_or(&Value::Null));
            let map_frame = localization.map_or(Some(origin), LocalFrame::from_localization);
            let (latitude, longitude) = if let Some(frame) = map_frame {
                frame.geographic(pose.x, pose.y).ok()?
            } else {
                // An unaligned phone must not appear at the default campus.
                let gps = raw_gps?;
                // Display the measured fix; it does not define an AR-to-geographic transform.
                (gps.latitude, gps.longitude)
            };
            let status = if rover.membership == "online"
                && rover.pose_age.is_some_and(|age| age < POSE_FRESH_SECONDS)
            {
                "online"
            } else if rover.membership == "online" {
                "pose stale"
            } else {
                "offline"
            };
            let report = decode_autonomy_value(&operator[&key]["authority"]).ok();
            let quality = reported_pose_quality(&operator[&key]);
            let outdoor = reported_outdoor(&operator[&key]);
            let hardware_value = &operator[&key]["hardware"];
            let hardware = HardwareObservation::from_status(hardware_value);
            let authority = &operator[&key]["authority"];
            let grid = occupancy(rover.id);
            let overlay_frame = map_frame.or_else(|| {
                // Without a heading, keep yaw as the marker draws it so the
                // grid still lines up with the rover model.
                let heading = outdoor
                    .as_ref()
                    .and_then(|report| report.heading_deg)
                    .unwrap_or_else(|| map_heading_degrees(pose.yaw));
                LocalFrame::pinned(latitude, longitude, pose.x, pose.y, pose.yaw, heading).ok()
            });

            Some(RoverMark {
                id: rover.id,
                latitude,
                longitude,
                map_frame,
                overlay_frame,
                heading: overlay_frame.map_or_else(
                    || map_heading_degrees(pose.yaw),
                    |frame| frame.heading_degrees(pose.yaw),
                ),
                local_x: pose.x,
                local_y: pose.y,
                yaw: pose.yaw,
                sequence: pose.sequence,
                status: status.into(),
                occupancy: grid
                    .as_ref()
                    .map(|(grid, age)| occupancy_summary(grid, *age))
                    .unwrap_or_else(|| "no occupancy".into()),
                grid: grid.as_ref().map(|(grid, _)| local_map(grid, 256)),
                requested_level: report.as_ref().map(|report| report.requested.clone()),
                effective_level: report.as_ref().and_then(|report| report.effective.clone()),
                recommended_level: report
                    .as_ref()
                    .and_then(|report| report.recommended.clone()),
                route: report
                    .as_ref()
                    .map(|report| report.route.clone())
                    .unwrap_or_default(),
                goal: goal_mark(rover, localizations, origin).map(|mut goal| {
                    // A local goal follows the grid it was picked on.
                    if goal.latitude.is_none()
                        && let Some(Ok((latitude, longitude))) =
                            overlay_frame.map(|frame| frame.geographic(goal.x, goal.y))
                    {
                        goal.latitude = Some(latitude);
                        goal.longitude = Some(longitude);
                    }
                    goal
                }),
                degraded: quality.as_ref().is_some_and(|quality| quality.degraded),
                degrade_reason: quality
                    .as_ref()
                    .map(|quality| quality.reason.clone())
                    .unwrap_or_default(),
                quality_reported: quality.is_some(),
                reversing: report
                    .as_ref()
                    .is_some_and(|report| report.commanded_linear.is_some_and(|speed| speed < 0.)),
                hold: report.as_ref().and_then(|report| report.hold.clone()),
                command_origin: report
                    .as_ref()
                    .map(|report| report.origin)
                    .unwrap_or_default(),
                arrival_radius: outdoor
                    .as_ref()
                    .map(|report| report.arrival_radius_m)
                    .unwrap_or(10.),
                outdoor_gps: outdoor
                    .as_ref()
                    .is_some_and(|report| report.acknowledged_outdoor()),
                hardware,
                authority_revision: authority["revision"].as_u64().unwrap_or(0),
                safety: authority["safety"].as_str().unwrap_or("").to_string(),
                run_id: report
                    .as_ref()
                    .and_then(|_| authority["run_id"].as_str())
                    .unwrap_or("")
                    .to_string(),
                navigation_revision: outdoor
                    .as_ref()
                    .map(|report| report.navigation_revision)
                    .unwrap_or(0),
                navigation_settings: authority["navigation"].clone(),
                mission_active: outdoor
                    .as_ref()
                    .is_some_and(|report| report.mission_active()),
                goal_active: rover
                    .goal
                    .as_ref()
                    .is_some_and(|goal| goal.state == GoalState::Active),
                app_update_required: argos_core::app_update_required(outdoor.as_ref()),
                raw_gps,
                pose_available: true,
                gps_display_freshness_seconds: display_policy(
                    localization.unwrap_or(&Value::Null),
                    &operator[&key]["authority"],
                )
                .gps_display_freshness_seconds,
                gps_source_fresh: localization.is_none_or(|report| report["gpsFresh"] != false),
                outdoor,
                exploration: report.as_ref().and_then(|_| decode_exploration(authority)),
            })
        })
        .collect()
}
fn display_policy(
    localization: &Value,
    authority: &Value,
) -> argos_core::ConnectivityPolicySettings {
    let policy = authority
        .get("connectivity")
        .and_then(|c| c.get("policy"))
        .or_else(|| localization.get("connectivity_policy"));
    policy
        .and_then(|p| {
            serde_json::from_value::<argos_core::ConnectivityPolicySettings>(p.clone()).ok()
        })
        .filter(|p| p.validate().is_ok())
        .unwrap_or_default()
}
fn display_link(snapshot: &Snapshot, localizations: &Value, operator: &Value) -> String {
    if !matches!(snapshot.link.as_str(), "healthy" | "stale") {
        return snapshot.link.clone();
    }
    let age_limit = snapshot
        .rovers
        .iter()
        .filter(|r| r.membership != "absent")
        .map(|r| {
            let id = r.id.to_string();
            display_policy(&localizations[&id], &operator[&id]["authority"])
                .fleet_display_stale_seconds
        })
        .reduce(f64::min)
        .unwrap_or(argos_core::ConnectivityPolicySettings::default().fleet_display_stale_seconds);
    if snapshot.fleet_age.is_some_and(|age| age >= age_limit) {
        "stale".into()
    } else {
        "healthy".into()
    }
}
fn retain_geographic_markers(
    snapshot: &Snapshot,
    localizations: &Value,
    previous: &FleetPicture,
    marks: &mut Vec<RoverMark>,
) {
    for rover in snapshot
        .rovers
        .iter()
        .filter(|r| r.membership != "absent" && r.pose.is_none())
    {
        if marks.iter().any(|m| m.id == rover.id) {
            continue;
        }
        let Some(gps) = decode_raw_gps(&localizations[rover.id.to_string()]) else {
            continue;
        };
        let Some(old) = previous
            .marks
            .iter()
            .find(|m| m.id == rover.id && m.raw_gps.is_some())
        else {
            continue;
        };
        let mut mark = old.clone();
        mark.latitude = gps.latitude;
        mark.longitude = gps.longitude;
        mark.raw_gps = Some(gps);
        mark.pose_available = false;
        mark.map_frame = None;
        mark.overlay_frame = None;
        mark.status = "pose unavailable".into();
        mark.grid = None;
        mark.goal = None;
        mark.route.clear();
        mark.gps_source_fresh = false;
        marks.push(mark);
    }
}

fn goal_mark(
    rover: &argos_core::RoverSnapshot,
    localizations: &Value,
    origin: LocalFrame,
) -> Option<GoalMark> {
    let goal = rover.goal.as_ref()?;
    if goal.state == GoalState::Idle {
        return None;
    }
    let (latitude, longitude) = if goal.latitude.is_some() && goal.longitude.is_some() {
        (goal.latitude, goal.longitude)
    } else {
        let pose = Pose {
            rover_id: rover.id,
            sequence: 0,
            x: goal.x,
            y: goal.y,
            yaw: 0.,
        };
        let localization = localizations.get(rover.id.to_string());
        match localization
            .map_or(Some(origin), LocalFrame::from_localization)
            .and_then(|frame| frame.geographic(pose.x, pose.y).ok())
        {
            Some((latitude, longitude)) => (Some(latitude), Some(longitude)),
            None => (None, None),
        }
    };
    let state = match goal.state {
        GoalState::Active => "active",
        GoalState::Arrived => "arrived",
        GoalState::Idle => "idle",
    };
    Some(GoalMark {
        x: goal.x,
        y: goal.y,
        latitude,
        longitude,
        state: state.into(),
    })
}
fn occupancy_summary(grid: &OccupancyGrid, age: f64) -> String {
    let occupied = grid
        .occupancy
        .iter()
        .filter(|cell| classify_cell(**cell) == CellClass::Occupied)
        .count();
    format!(
        "{}x{}, {occupied} occupied, {age:.1}s",
        grid.width, grid.height
    )
}
pub fn describe(snapshot: &Snapshot, _marks: &[RoverMark]) -> String {
    let count = snapshot
        .rovers
        .iter()
        .filter(|r| r.membership != "absent")
        .map(|r| r.id)
        .collect::<BTreeSet<_>>()
        .len();
    let noun = if count == 1 { "rover" } else { "rovers" };
    let mut detail = match snapshot.link.as_str() {
        "healthy" => format!("{count} {noun}"),
        "connecting" => "Waiting for fleet state".into(),
        "stale" => format!("Fleet state is stale · {count} {noun}"),
        "disconnected" => "Disconnected".into(),
        other => other.to_string(),
    };
    if let Some(notice) = &snapshot.notice {
        detail = format!("{detail} · {notice}");
    }
    detail
}
/// Centroid to fly to when the set of positioned rovers changes.
///
/// An empty fleet returns `None` so the camera stays on the start location.
/// The same id set returns `None` so later pose ticks do not recenter.
pub fn framing_target(
    previous: &BTreeSet<u64>,
    marks: &[RoverMark],
) -> Option<(BTreeSet<u64>, f64, f64)> {
    let ids: BTreeSet<u64> = marks.iter().map(|mark| mark.id).collect();
    if ids.is_empty() || &ids == previous {
        return None;
    }
    let count = marks.len() as f64;
    let latitude = marks.iter().map(|mark| mark.latitude).sum::<f64>() / count;
    let longitude = marks.iter().map(|mark| mark.longitude).sum::<f64>() / count;
    Some((ids, latitude, longitude))
}

/// Reframe after late geographic alignment or meaningful rover movement.
/// Keep small pose/GPS fluctuations from restarting the camera animation.
pub fn reframing_target(
    previous: &BTreeSet<u64>,
    center: Option<(f64, f64)>,
    marks: &[RoverMark],
) -> Option<(BTreeSet<u64>, f64, f64)> {
    if let Some(target) = framing_target(previous, marks) {
        return Some(target);
    }
    let live: Vec<_> = marks
        .iter()
        .filter(|mark| mark.status == "online")
        .collect();
    if live.is_empty() {
        return None;
    }
    let latitude = live.iter().map(|mark| mark.latitude).sum::<f64>() / live.len() as f64;
    let longitude = live.iter().map(|mark| mark.longitude).sum::<f64>() / live.len() as f64;
    if center
        .is_none_or(|(lat, lon)| argos_core::geodesic_metres(lat, lon, latitude, longitude) >= 20.)
    {
        Some((
            marks.iter().map(|mark| mark.id).collect(),
            latitude,
            longitude,
        ))
    } else {
        None
    }
}

/// Process-lifetime subscriber. Drop does not stop the thread.
#[derive(Clone)]
pub struct FleetFeed {
    picture: Arc<Mutex<FleetPicture>>,
    client: Arc<Mutex<Option<Arc<Client>>>>,
    limits: Arc<Mutex<TeleopLimits>>,
    origin: LocalFrame,
    /// The rover whose heavy topics the poll loop keeps subscribed.
    focus: Arc<Mutex<Option<u64>>>,
}
impl FleetFeed {
    pub fn start(config: DashboardConfig) -> Self {
        let limits = Arc::new(Mutex::new(config.teleop));
        if let Some(notice) = &config.connect_notice {
            return Self {
                picture: Arc::new(Mutex::new(FleetPicture {
                    link: "password".into(),
                    detail: notice.clone(),
                    command: String::new(),
                    marks: Vec::new(),
                    presence: Vec::new(),
                    observed_at: std::time::Instant::now(),
                })),
                client: Arc::new(Mutex::new(None)),
                limits,
                origin: config.origin,
                focus: Arc::new(Mutex::new(None)),
            };
        }
        let origin = config.origin;
        let picture = Arc::new(Mutex::new(FleetPicture {
            link: "connecting".into(),
            detail: format!("Connecting to {} ({})", config.endpoint, config.prefix),
            command: String::new(),
            marks: Vec::new(),
            presence: Vec::new(),
            observed_at: std::time::Instant::now(),
        }));
        let client = Arc::new(Mutex::new(None));
        let focus = Arc::new(Mutex::new(None));
        let shared = picture.clone();
        let slot = client.clone();
        let wanted = focus.clone();
        std::thread::Builder::new()
            .name("argos-fleet".into())
            .spawn(move || run(config, shared, slot, wanted))
            .expect("argos-fleet thread");
        Self {
            picture,
            client,
            limits,
            origin,
            focus,
        }
    }
    /// Keep this rover's occupancy grid, point cloud, and depth subscribed,
    /// or none. Applied on the next poll and again after a reconnect.
    pub fn focus(&self, id: Option<u64>) {
        *self.focus.lock().unwrap() = id;
    }
    pub fn origin(&self) -> LocalFrame {
        self.origin
    }
    pub fn picture(&self) -> FleetPicture {
        let mut picture = self.picture.lock().unwrap().clone();
        let elapsed = picture.observed_at.elapsed().as_secs_f64();
        for mark in &mut picture.marks {
            if let Some(gps) = &mut mark.raw_gps {
                gps.age_s = gps.age_s.map(|age| age + elapsed);
            }
        }
        for row in &mut picture.presence {
            row.observation_age = row.observation_age.map(|age| age + elapsed);
            if let Some(mark) = picture.marks.iter().find(|mark| mark.id == row.id) {
                row.position_reason = position_label(mark);
            }
            if !row.source_reason.is_empty() {
                row.position_reason.push_str(" · ");
                row.position_reason.push_str(&row.source_reason);
            }
        }
        picture
    }
    pub fn limits(&self) -> TeleopLimits {
        *self.limits.lock().unwrap()
    }
    pub fn set_linear(&self, linear: f64) -> Result<(), String> {
        self.update_limits(|limits| limits.linear = linear)
    }
    pub fn set_spin(&self, allow_spin: bool) -> Result<(), String> {
        self.update_limits(|limits| limits.allow_spin = allow_spin)
    }
    pub fn set_yaw_rate(&self, yaw_rate: f64) -> Result<(), String> {
        self.update_limits(|limits| limits.yaw_rate = yaw_rate)
    }
    pub fn set_yaw_accel(&self, yaw_accel: f64) -> Result<(), String> {
        self.update_limits(|limits| limits.yaw_accel = yaw_accel)
    }
    pub fn set_yaw_decel(&self, yaw_decel: f64) -> Result<(), String> {
        self.update_limits(|limits| limits.yaw_decel = yaw_decel)
    }
    pub fn set_min_radius(&self, min_turn_radius: f64) -> Result<(), String> {
        self.update_limits(|limits| limits.min_turn_radius = min_turn_radius)
    }
    fn update_limits(&self, edit: impl FnOnce(&mut TeleopLimits)) -> Result<(), String> {
        let mut limits = self.limits.lock().unwrap();
        edit(&mut limits);
        let checked = (*limits).validate().map_err(|error| error.to_string())?;
        *limits = checked;
        if let Some(client) = self.client.lock().unwrap().clone() {
            client.set_teleop_guard(*limits);
        }
        Ok(())
    }
    pub fn publish_level(&self, id: u64, level: &str) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        let level = level.to_string();
        std::thread::spawn(move || {
            let message = (|| {
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                let token = format!("gpui-{id}-{}", unix_millis());
                let bytes = argos_core::encode_level_request(&level, &token)
                    .map_err(|error| error.to_string())?;
                let payload = String::from_utf8(bytes).map_err(|_| "level payload".to_string())?;
                client
                    .operator_command(id, "autonomy", &payload)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(format!(
                    "Requested {level} for rover {id} · waiting for vehicle acknowledgement"
                ))
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    /// Start L4 frontier exploration for `budget_seconds`. Terra picks and
    /// drives the frontiers; this only sends the level with its budget.
    pub fn publish_explore(&self, id: u64, budget_seconds: f64) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        std::thread::spawn(move || {
            let message = (|| {
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                let token = format!("gpui-{id}-{}", unix_millis());
                let bytes = argos_core::encode_explore_request(&token, budget_seconds)
                    .map_err(|error| error.to_string())?;
                let payload = String::from_utf8(bytes).map_err(|_| "level payload".to_string())?;
                client
                    .operator_command(id, "autonomy", &payload)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(format!(
                    "Requested L4 Explore for {:.0} min on rover {id} · waiting for vehicle acknowledgement",
                    budget_seconds / 60.
                ))
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    /// Publish a confirmed goal. A geographic goal is admitted only while outdoor
    /// readiness still matches the reviewed run and navigation revision.
    pub fn publish_goal(&self, id: u64, goal: GoalRequest, arrival_radius_m: Option<f64>) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        std::thread::spawn(move || {
            let message = (|| {
                let mut review = None;
                {
                    let current = picture.lock().unwrap();
                    if current.link != "healthy" {
                        return Err("Link is down".to_string());
                    }
                    let mark = current
                        .marks
                        .iter()
                        .find(|mark| mark.id == id)
                        .ok_or("Select a rover that is in the fleet")?;
                    if let GoalRequest::Geographic {
                        latitude,
                        longitude,
                        ..
                    } = goal
                    {
                        if mark.app_update_required {
                            return Err(APP_UPDATE_REQUIRED.into());
                        }
                        let level = mark
                            .effective_level
                            .as_deref()
                            .or(mark.requested_level.as_deref());
                        admit_outdoor_goal(OutdoorAdmission {
                            report: mark.outdoor.as_ref(),
                            latitude,
                            longitude,
                            arrival_radius_m: arrival_radius_m.unwrap_or(mark.arrival_radius),
                            level,
                            run_id: &mark.run_id,
                            navigation_revision: mark.navigation_revision,
                        })
                        .map_err(str::to_string)?;
                    }
                    if matches!(goal, GoalRequest::Geographic { .. }) {
                        review = Some((
                            mark.run_id.clone(),
                            mark.navigation_revision,
                            arrival_radius_m.unwrap_or(mark.arrival_radius),
                        ));
                    }
                    if let Some(reason) = goal_blocked(mark.degraded) {
                        return Err(format!("{reason} ({})", mark.degrade_reason));
                    }
                    if let Some(reason) = drive_block(&mark.hardware) {
                        return Err(reason.into());
                    }
                }
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                client
                    .send_reviewed_goal(id, goal, review)
                    .map_err(|error| error.to_string())?;
                Ok("Goal sent".to_string())
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    pub fn select_source(&self, id: u64, source: &str) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        let source = source.to_string();
        std::thread::spawn(move || {
            let message = (|| {
                let (run_id, revision, ready_and_neutral, goal_active, mission_active, outdoor) = {
                    let current = picture.lock().unwrap();
                    let mark = current
                        .marks
                        .iter()
                        .find(|mark| mark.id == id)
                        .ok_or("Select a rover that is in the fleet")?;
                    (
                        mark.run_id.clone(),
                        mark.navigation_revision,
                        mark.hardware.ready_and_neutral(),
                        mark.goal_active,
                        mark.mission_active,
                        mark.outdoor.clone(),
                    )
                };
                if let Some(reason) = source_change_refusal(
                    ready_and_neutral,
                    goal_active,
                    mission_active,
                    &source,
                    outdoor.as_ref(),
                ) {
                    return Err(reason.to_string());
                }
                let token = format!("src-{id}-{}", unix_millis());
                let bytes = encode_source_request(&source, &token, &run_id, revision)
                    .map_err(|error| error.to_string())?;
                let payload =
                    String::from_utf8(bytes).map_err(|_| "navigation source".to_string())?;
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                client
                    .operator_command(id, "navigation/source", &payload)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(format!("Requested {source} navigation"))
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    /// Ask Terra to allow (or forbid) autonomous in-place turns, and set
    /// CrabTrapBot's body size. Terra applies it only with neutral hardware,
    /// no active goal, and current authority.
    pub fn configure_navigation(&self, id: u64, allow_in_place: bool) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        std::thread::spawn(move || {
            let message = (|| {
                let (navigation, run_id, authority_revision) = {
                    let current = picture.lock().unwrap();
                    let mark = current
                        .marks
                        .iter()
                        .find(|mark| mark.id == id)
                        .ok_or("Select a rover that is in the fleet")?;
                    if !mark.hardware.ready_and_neutral() {
                        return Err("Settings need ready hardware with neutral output".into());
                    }
                    if mark.goal_active {
                        return Err("Cancel the active goal before changing settings".into());
                    }
                    (
                        mark.navigation_settings.clone(),
                        mark.run_id.clone(),
                        mark.authority_revision,
                    )
                };
                let token = format!("nav-{id}-{}", unix_millis());
                let bytes = encode_navigation_settings(
                    &navigation,
                    NavigationEdit::crab_trap_bot(allow_in_place),
                    &token,
                    &run_id,
                    authority_revision,
                )
                .map_err(|error| error.to_string())?;
                let payload =
                    String::from_utf8(bytes).map_err(|_| "navigation settings".to_string())?;
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                client
                    .operator_command(id, "navigation/settings", &payload)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(format!(
                    "Requested autonomy in-place turns {} with CrabTrapBot size",
                    if allow_in_place { "on" } else { "off" }
                ))
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    pub fn resume_outdoor(&self, id: u64) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let current = picture.lock().unwrap();
                let mark = current
                    .marks
                    .iter()
                    .find(|m| m.id == id)
                    .ok_or("Rover unavailable")?;
                let payload=serde_json::json!({"schema_version":1,"token":format!("resume-{id}-{}",unix_millis()),"run_id":mark.run_id,"navigation_revision":mark.navigation_revision}).to_string();
                drop(current);
                let guard = client.lock().unwrap();
                let c = guard.as_ref().ok_or("Not connected")?;
                c.operator_command(id, "navigation/resume", &payload)
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>("Outdoor resume requested".into())
            })();
            picture.lock().unwrap().command = result.unwrap_or_else(|e| e);
        });
    }
    pub fn cancel_goal(&self, id: u64) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        std::thread::spawn(move || {
            let message = (|| {
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                client.cancel_goal(id).map_err(|error| error.to_string())?;
                Ok::<_, String>(format!("Cancel sent to rover {id}"))
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
    }
    pub fn begin_drive(&self, id: u64) -> Result<(), String> {
        {
            let current = self.picture.lock().unwrap();
            if current.link != "healthy" {
                return Err("Link is down".into());
            }
            let mark = current
                .marks
                .iter()
                .find(|mark| mark.id == id)
                .ok_or("Rover is not in the fleet")?;
            if !mark.control_pose_ready() {
                return Err("Pose is stale".into());
            }
            if let Some(reason) = drive_block(&mark.hardware) {
                return Err(reason.into());
            }
        }
        let client = self.connected()?;
        client.set_teleop_guard(*self.limits.lock().unwrap());
        Ok(())
    }
    pub fn end_drive(&self) {
        if let Ok(client) = self.connected() {
            client.clear_input();
            client.clear_teleop_guard();
        }
    }
    pub fn drive_key(&self, id: u64, key: &str, down: bool) -> Result<(), String> {
        if down {
            let current = self.picture.lock().unwrap();
            if current.link != "healthy" {
                return Err("Link is down".into());
            }
            let mark = current
                .marks
                .iter()
                .find(|mark| mark.id == id)
                .ok_or("Rover is not in the fleet")?;
            if !mark.control_pose_ready() {
                return Err("Pose is stale".into());
            }
            if let Some(reason) = drive_block(&mark.hardware) {
                return Err(reason.into());
            }
        }
        let client = self.connected()?;
        client
            .input(id, key, down)
            .map_err(|error| error.to_string())
    }
    pub fn refresh_drive(&self, id: u64) {
        if let Ok(client) = self.connected() {
            client.refresh_drive(id);
        }
    }
    pub fn safety_reset(&self, id: u64) -> Result<(), String> {
        let client = self.connected()?;
        client
            .action(id, "safety", serde_json::json!({"action": "reset"}))
            .map_err(|error| error.to_string())?;
        self.picture.lock().unwrap().command = format!("Reset sent to rover {id}");
        Ok(())
    }
    pub fn emergency_stop(&self, id: u64) -> Result<(), String> {
        let client = self.connected()?;
        client.clear_teleop_guard();
        client
            .action(id, "safety", serde_json::json!({"action": "stop"}))
            .map_err(|error| error.to_string())?;
        self.picture.lock().unwrap().command = format!("Stop sent to rover {id}");
        Ok(())
    }
    fn connected(&self) -> Result<Arc<Client>, String> {
        self.client
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "Not connected".to_string())
    }
}
fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}
fn run(
    config: DashboardConfig,
    picture: Arc<Mutex<FleetPicture>>,
    slot: Arc<Mutex<Option<Arc<Client>>>>,
    focus: Arc<Mutex<Option<u64>>>,
) {
    let mut history = LocalizationHistory::default();
    let started = std::time::Instant::now();
    loop {
        let credentials = config
            .credentials
            .as_ref()
            .map(|(user, password)| (user.as_str(), password.as_str()));
        match Client::connect_authenticated(&config.endpoint, &config.prefix, credentials) {
            Ok(client) => {
                let client = Arc::new(client);
                *slot.lock().unwrap() = Some(client.clone());
                poll(&client, &config, &picture, &focus, &mut history, started);
                mark_disconnected(&picture);
                *slot.lock().unwrap() = None;
            }
            Err(error) => {
                let mut current = picture.lock().unwrap();
                current.link = "disconnected".into();
                current.detail = error.to_string();
                for mark in &mut current.marks {
                    mark.status = "offline".into();
                    mark.gps_source_fresh = false;
                }
                for row in &mut current.presence {
                    row.membership = "offline".into();
                }
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
fn mark_disconnected(picture: &Mutex<FleetPicture>) {
    let mut current = picture.lock().unwrap();
    current.link = "disconnected".into();
    current.detail = format!(
        "Disconnected · {} last known rovers",
        current.presence.len()
    );
    for mark in &mut current.marks {
        mark.status = "offline".into();
        mark.gps_source_fresh = false;
    }
    for row in &mut current.presence {
        row.membership = "offline".into();
    }
}
fn poll(
    client: &Client,
    config: &DashboardConfig,
    picture: &Mutex<FleetPicture>,
    focus: &Mutex<Option<u64>>,
    history: &mut LocalizationHistory,
    started: std::time::Instant,
) {
    let mut watches: BTreeMap<u64, PoseWatch> = BTreeMap::new();
    let issued_after_ms = unix_millis();
    let mut last_receipts: BTreeMap<u64, String> = BTreeMap::new();
    loop {
        // A failed declare is retried on the next pass.
        let _ = client.focus(*focus.lock().unwrap());
        let mut snapshot = client.snapshot();
        if snapshot.link == "failed" || snapshot.link == "disconnected" {
            break;
        }
        let localizations =
            serde_json::from_str::<Value>(&client.localization_snapshot()).unwrap_or(Value::Null);
        if snapshot.fleet_age.is_none() {
            // A replacement client has not received authoritative membership yet.
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        let ids = snapshot
            .rovers
            .iter()
            .filter(|r| r.membership != "absent")
            .map(|r| r.id)
            .collect();
        history.observe_membership(
            &snapshot
                .rovers
                .iter()
                .map(|r| (r.id, r.membership.as_str()))
                .collect::<Vec<_>>(),
            started.elapsed().as_secs_f64(),
        );
        history.retain(&ids);
        let operator =
            serde_json::from_str::<Value>(&client.operator_state_snapshot()).unwrap_or(Value::Null);
        history.scope_runs(&operator);
        let mut localizations = localizations;
        if let Some(reports) = localizations.as_object_mut() {
            for (id, report) in reports {
                *report = scope_localization(
                    report.clone(),
                    operator[id]["authority"]["run_id"].as_str(),
                );
            }
        }
        let localizations = history.update(&localizations, started.elapsed().as_secs_f64());
        history.retain(&ids);
        let mut marks = fleet_marks(
            &snapshot,
            &localizations,
            &operator,
            |id| client.occupancy_grid(id).map(|(grid, age, _)| (grid, age)),
            config.origin,
        );
        snapshot.link = display_link(&snapshot, &localizations, &operator);
        retain_geographic_markers(
            &snapshot,
            &localizations,
            &picture.lock().unwrap(),
            &mut marks,
        );
        let now = started.elapsed().as_secs_f64();
        for mark in &mut marks {
            if mark.quality_reported {
                continue;
            }
            let (degraded, reason) = watches.entry(mark.id).or_default().observe(
                mark.yaw,
                mark.sequence,
                now,
                client.pose_tracking(mark.id),
                config.degrade,
            );
            mark.degraded = degraded;
            mark.degrade_reason = if degraded {
                fallback_degrade_reason(reason)
            } else {
                String::new()
            };
        }
        {
            let mut current = picture.lock().unwrap();
            if let Some(entries) = operator.as_object() {
                for (id, state) in entries {
                    let Ok(id) = id.parse::<u64>() else { continue };
                    if let Some((receipt, message)) =
                        argos_core::vehicle_request_receipt(&state["authority"], issued_after_ms)
                        && last_receipts.get(&id) != Some(&receipt)
                    {
                        last_receipts.insert(id, receipt);
                        current.command = format!("Rover {id}: {message}");
                    }
                }
            }
            current.detail = describe(&snapshot, &marks);
            current.presence = fleet_presence(&snapshot, &marks);
            for row in &mut current.presence {
                if matches!(row.membership.as_str(), "online" | "stale") {
                    let id = row.id.to_string();
                    let limit = display_policy(&localizations[&id], &operator[&id]["authority"])
                        .fleet_display_stale_seconds;
                    row.membership = if snapshot.fleet_age.is_some_and(|age| age >= limit) {
                        "stale".into()
                    } else {
                        "online".into()
                    };
                }
                row.source_reason = localizations[row.id.to_string()]["reason"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
            }
            current.link = snapshot.link;
            current.marks = marks;
            current.observed_at = std::time::Instant::now();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/fleet_tests.rs"]
mod tests;
