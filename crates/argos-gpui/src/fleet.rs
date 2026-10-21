//! Background Zenoh subscription and the commands the selected rover may receive.
//!
//! Connect and snapshot polling run on a thread named `argos-fleet`. Publishing
//! stays on that client and is idle until the window selects a rover.

use argos_core::{
    APP_UPDATE_REQUIRED, CellClass, CommandOrigin, DegradeLimits, GoalRequest, GoalState,
    HardwareCommand, HardwareObservation, LocalFrame, LocalMap, OccupancyGrid, OutdoorAdmission,
    OutdoorReport, Pose, PoseWatch, RawGps, RouteSample, Snapshot, TeleopLimits, Topics,
    admit_outdoor_goal, classify_cell, decode_autonomy_value, decode_raw_gps, drive_block,
    encode_hardware, encode_source_request, fallback_degrade_reason, goal_blocked, local_map,
    map_heading_degrees, reported_outdoor, reported_pose_quality, source_change_refusal,
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
const SETTING_KEYS: [&str; 17] = [
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
fn dotenv_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(root) = manifest.parent().and_then(Path::parent) {
        paths.push(root.join(".env"));
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
    pub disarmed: bool,
    pub hardware: HardwareObservation,
    pub authority_revision: u64,
    pub safety: String,
    pub run_id: String,
    pub navigation_revision: u64,
    pub outdoor: Option<OutdoorReport>,
    pub raw_gps: Option<RawGps>,
    pub app_update_required: bool,
    pub mission_active: bool,
    pub goal_active: bool,
}
impl RoverMark {
    /// L0 uses direct operator effort; a missing camera pose is only a map warning.
    pub fn control_pose_ready(&self) -> bool {
        self.status == "online"
            || (self.status == "pose stale" && self.requested_level.as_deref() == Some("teleop"))
    }
}

#[derive(Clone, Debug)]
pub struct FleetPicture {
    pub link: String,
    pub detail: String,
    pub command: String,
    pub marks: Vec<RoverMark>,
}
impl Default for FleetPicture {
    fn default() -> Self {
        Self {
            link: "connecting".into(),
            detail: "Connecting".into(),
            command: String::new(),
            marks: Vec::new(),
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

            Some(RoverMark {
                id: rover.id,
                latitude,
                longitude,
                map_frame,
                heading: map_heading_degrees(pose.yaw),
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
                goal: goal_mark(rover, localizations, origin),
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
                disarmed: hardware.present && !hardware.armed && !hardware.arming,
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
                mission_active: outdoor
                    .as_ref()
                    .is_some_and(|report| report.mission_active()),
                goal_active: rover
                    .goal
                    .as_ref()
                    .is_some_and(|goal| goal.state == GoalState::Active),
                app_update_required: argos_core::app_update_required(outdoor.as_ref()),
                raw_gps,
                outdoor,
            })
        })
        .collect()
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
pub fn describe(snapshot: &Snapshot, marks: &[RoverMark]) -> String {
    let count = marks.len();
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
                })),
                client: Arc::new(Mutex::new(None)),
                limits,
                origin: config.origin,
            };
        }
        let origin = config.origin;
        let picture = Arc::new(Mutex::new(FleetPicture {
            link: "connecting".into(),
            detail: format!("Connecting to {} ({})", config.endpoint, config.prefix),
            command: String::new(),
            marks: Vec::new(),
        }));
        let client = Arc::new(Mutex::new(None));
        let shared = picture.clone();
        let slot = client.clone();
        std::thread::Builder::new()
            .name("argos-fleet".into())
            .spawn(move || run(config, shared, slot))
            .expect("argos-fleet thread");
        Self {
            picture,
            client,
            limits,
            origin,
        }
    }
    pub fn origin(&self) -> LocalFrame {
        self.origin
    }
    pub fn picture(&self) -> FleetPicture {
        self.picture.lock().unwrap().clone()
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
                Ok::<_, String>(format!("Sent {level} to rover {id}"))
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
                let (run_id, revision, disarmed, goal_active, mission_active, outdoor) = {
                    let current = picture.lock().unwrap();
                    let mark = current
                        .marks
                        .iter()
                        .find(|mark| mark.id == id)
                        .ok_or("Select a rover that is in the fleet")?;
                    (
                        mark.run_id.clone(),
                        mark.navigation_revision,
                        mark.disarmed,
                        mark.goal_active,
                        mark.mission_active,
                        mark.outdoor.clone(),
                    )
                };
                if let Some(reason) = source_change_refusal(
                    disarmed,
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
    pub fn publish_hardware(&self, id: u64, command: HardwareCommand) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        std::thread::spawn(move || {
            let message = (|| {
                let bytes = encode_hardware(&command).map_err(|error| error.to_string())?;
                let payload =
                    String::from_utf8(bytes).map_err(|_| "hardware payload".to_string())?;
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                client
                    .operator_command(id, "hardware", &payload)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(match command {
                    HardwareCommand::Arm { .. } => format!("Arm request sent to rover {id}"),
                    HardwareCommand::Disarm { .. } => format!("Disarm sent to rover {id}"),
                    HardwareCommand::Reset => "Reset is a safety action".into(),
                })
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
        });
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
) {
    loop {
        let credentials = config
            .credentials
            .as_ref()
            .map(|(user, password)| (user.as_str(), password.as_str()));
        match Client::connect_authenticated(&config.endpoint, &config.prefix, credentials) {
            Ok(client) => {
                let client = Arc::new(client);
                *slot.lock().unwrap() = Some(client.clone());
                poll(&client, &config, &picture);
                *slot.lock().unwrap() = None;
            }
            Err(error) => {
                let mut current = picture.lock().unwrap();
                current.link = "disconnected".into();
                current.detail = error.to_string();
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
fn poll(client: &Client, config: &DashboardConfig, picture: &Mutex<FleetPicture>) {
    let mut watches: BTreeMap<u64, PoseWatch> = BTreeMap::new();
    let started = std::time::Instant::now();
    loop {
        let snapshot = client.snapshot();
        if snapshot.link == "failed" || snapshot.link == "disconnected" {
            break;
        }
        let localizations =
            serde_json::from_str::<Value>(&client.localization_snapshot()).unwrap_or(Value::Null);
        let operator =
            serde_json::from_str::<Value>(&client.operator_state_snapshot()).unwrap_or(Value::Null);
        let mut marks = fleet_marks(
            &snapshot,
            &localizations,
            &operator,
            |id| client.occupancy_grid(id).map(|(grid, age, _)| (grid, age)),
            config.origin,
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
            current.detail = describe(&snapshot, &marks);
            current.link = snapshot.link;
            current.marks = marks;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/fleet_tests.rs"]
mod tests;
