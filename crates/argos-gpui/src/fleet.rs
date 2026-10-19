//! Background Zenoh subscription and the commands the selected rover may receive.
//!
//! Connect and snapshot polling run on a thread named `argos-fleet`. Publishing
//! stays on that client and is idle until the window selects a rover.

use argos_core::{
    CellClass, CommandOrigin, DegradeLimits, GoalRequest, GoalState, LocalFrame, LocalMap,
    OccupancyGrid, Pose, PoseWatch, RouteSample, Snapshot, TeleopLimits, Topics, classify_cell,
    decode_autonomy_value, fallback_degrade_reason, geographic_goal_refusal, goal_blocked,
    local_map, map_heading_degrees, outdoor_gps_advertised, reported_pose_quality,
    rover_geographic,
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
const POSE_FRESH_SECONDS: f64 = 2.5;
const SETTING_KEYS: [&str; 16] = [
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
];

#[derive(Clone, Debug)]
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
}
impl DashboardConfig {
    pub fn from_env() -> Result<Self, String> {
        let mut process = BTreeMap::new();
        for key in SETTING_KEYS {
            if std::env::var_os(key).is_some() {
                process.insert(key.to_string(), std::env::var(key).unwrap_or_default());
            }
        }
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
        })
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
    /// Metres. Provisional outdoor arrival radius from navigation telemetry.
    pub arrival_radius: f64,
    /// True only when navigation telemetry advertises `outdoor_gps`.
    pub outdoor_gps: bool,
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
            let (latitude, longitude) =
                rover_geographic(pose, localizations.get(&key), origin).ok()?;
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
            let grid = occupancy(rover.id);
            Some(RoverMark {
                id: rover.id,
                latitude,
                longitude,
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
                grid: grid.as_ref().map(|(grid, _)| local_map(grid, 28)),
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
                arrival_radius: report
                    .as_ref()
                    .map(|report| report.arrival_radius)
                    .unwrap_or(5.),
                outdoor_gps: outdoor_gps_advertised(&operator[&key]),
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
        match rover_geographic(&pose, localizations.get(rover.id.to_string()), origin) {
            Ok((latitude, longitude)) => (Some(latitude), Some(longitude)),
            Err(_) => (None, None),
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
    /// Publish a confirmed goal. Local is the default. Geographic is refused
    /// unless this rover's telemetry advertises `outdoor_gps`.
    pub fn publish_goal(&self, id: u64, goal: GoalRequest) {
        let client = self.client.clone();
        let picture = self.picture.clone();
        std::thread::spawn(move || {
            let message = (|| {
                let geographic = matches!(goal, GoalRequest::Geographic { .. });
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
                    if let Some(reason) = geographic_goal_refusal(mark.outdoor_gps)
                        && geographic
                    {
                        return Err(reason.to_string());
                    }
                    if let Some(reason) = goal_blocked(mark.degraded) {
                        return Err(format!("{reason} ({})", mark.degrade_reason));
                    }
                }
                let guard = client.lock().unwrap();
                let client = guard.as_ref().ok_or("Not connected")?;
                client
                    .send_goal(id, goal)
                    .map_err(|error| error.to_string())?;
                Ok("Goal sent".to_string())
            })();
            picture.lock().unwrap().command = message.unwrap_or_else(|error| error);
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
            if mark.status != "online" {
                return Err("Pose is stale".into());
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
            if mark.status != "online" {
                return Err("Pose is stale".into());
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
mod tests {
    use super::*;
    use argos_core::{Anchor, OccupancyGrid, Pose, RoverSnapshot};

    fn pose(x: f64, y: f64, yaw: f64) -> Pose {
        Pose {
            rover_id: 3,
            sequence: 0,
            x,
            y,
            yaw,
        }
    }
    fn rover(
        id: u64,
        membership: &str,
        pose: Option<Pose>,
        pose_age: Option<f64>,
    ) -> RoverSnapshot {
        RoverSnapshot {
            id,
            membership: membership.into(),
            pose,
            pose_age,
            goal: None,
            goal_age: None,
            goal_received_at: None,
            command_phase: "none".into(),
            command_token: None,
        }
    }

    #[test]
    fn unset_env_defaults_to_the_hosted_fleet_and_asks_for_a_password() {
        let config =
            DashboardConfig::parse(None, None, None, None, (None, None), (None, None)).unwrap();
        assert_eq!(config.endpoint, DEFAULT_ENDPOINT);
        assert_eq!(config.prefix, DEFAULT_PREFIX);
        assert!(config.credentials.is_none());
        assert_eq!(config.connect_notice.as_deref(), Some(PASSWORD_NOTICE));
        assert_eq!(config.start_latitude, DEFAULT_START_LATITUDE);
        assert_eq!(config.start_longitude, DEFAULT_START_LONGITUDE);
        assert_eq!(config.origin.latitude, DEFAULT_START_LATITUDE);
        let picture = FleetFeed::start(config).picture();
        assert_eq!(picture.link, "password");
        assert_eq!(picture.detail, PASSWORD_NOTICE);
        assert!(picture.marks.is_empty());
    }

    #[test]
    fn password_from_the_environment_is_kept_and_local_zorvane_clears_the_user() {
        let hosted = DashboardConfig::parse(
            None,
            None,
            None,
            Some("from-the-operator"),
            (None, None),
            (None, None),
        )
        .unwrap();
        assert_eq!(
            hosted.credentials,
            Some((DEFAULT_USER.into(), "from-the-operator".into()))
        );
        assert!(hosted.connect_notice.is_none());
        let local = DashboardConfig::parse(
            Some("tcp/127.0.0.1:7447"),
            Some("terra/rover"),
            Some(""),
            Some(""),
            (None, None),
            (None, None),
        )
        .unwrap();
        assert_eq!(local.endpoint, "tcp/127.0.0.1:7447");
        assert_eq!(local.prefix, "terra/rover");
        assert!(local.credentials.is_none());
        assert!(local.connect_notice.is_none());
    }

    #[test]
    fn dotenv_yields_to_the_environment_and_the_current_directory_overrides_the_workspace() {
        let dir = std::env::temp_dir().join(format!("argos-dotenv-{}", std::process::id()));
        let root = dir.join("root");
        let cwd = dir.join("cwd");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(
            root.join(".env"),
            "ARGOS_ZENOH_ENDPOINT=tls/from-workspace:7448\nARGOS_ZENOH_PREFIX=terra/phone\nARGOS_ZENOH_PASSWORD=from-workspace\n",
        )
        .unwrap();
        std::fs::write(
            cwd.join(".env"),
            "ARGOS_ZENOH_PREFIX=terra/rover\nARGOS_ZENOH_USER=local\n",
        )
        .unwrap();
        assert!(read_dotenv(dir.join("missing.env").as_path()).is_empty());
        let process = BTreeMap::from([("ARGOS_ZENOH_ENDPOINT".into(), "tcp/from-env:1".into())]);
        let merged = merge_dotenv(
            &process,
            &[
                read_dotenv(&root.join(".env")),
                read_dotenv(&cwd.join(".env")),
            ],
        );
        let value = |key: &str| {
            process
                .get(key)
                .or_else(|| merged.get(key))
                .map(String::as_str)
        };
        assert_eq!(value("ARGOS_ZENOH_ENDPOINT"), Some("tcp/from-env:1"));
        assert_eq!(value("ARGOS_ZENOH_PREFIX"), Some("terra/rover"));
        assert_eq!(value("ARGOS_ZENOH_USER"), Some("local"));
        assert_eq!(value("ARGOS_ZENOH_PASSWORD"), Some("from-workspace"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn config_rejects_partial_pairs_and_bad_numbers() {
        assert!(
            DashboardConfig::parse(
                Some("udp/127.0.0.1:1"),
                None,
                None,
                None,
                (None, None),
                (None, None)
            )
            .is_err()
        );
        assert!(
            DashboardConfig::parse(
                Some("tcp/127.0.0.1:7447"),
                None,
                Some("terra"),
                None,
                (None, None),
                (None, None)
            )
            .is_err()
        );
        assert!(
            DashboardConfig::parse(None, None, None, None, (Some("nope"), None), (None, None))
                .is_err()
        );
        assert!(
            DashboardConfig::parse(None, None, None, None, (None, None), (Some("10"), None))
                .is_err()
        );
        let config = DashboardConfig::parse(
            None,
            Some("terra/phone"),
            None,
            None,
            (Some("10"), Some("20")),
            (Some("1"), Some("2")),
        )
        .unwrap();
        assert_eq!(config.prefix, "terra/phone");
        assert_eq!(config.start_latitude, 10.);
        assert_eq!(config.origin.latitude, 1.);
        assert_eq!(config.origin.longitude, 2.);
    }

    #[test]
    fn marks_follow_localization_and_summarize_occupancy() {
        let origin = LocalFrame::origin(0., 0.).unwrap();
        let mut pose = pose(12., 4., std::f64::consts::FRAC_PI_2);
        pose.rover_id = 3;
        let snapshot = Snapshot {
            link: "healthy".into(),
            fleet_age: Some(0.1),
            rovers: vec![
                rover(3, "online", Some(pose), Some(0.2)),
                rover(4, "online", None, None),
            ],
            notice: None,
        };
        let localizations = serde_json::json!({
            "3": {
                "version": 1,
                "mode": "geographic",
                "originLatitude": 38.8297,
                "originLongitude": -77.3075
            }
        });
        let grid = OccupancyGrid {
            schema_version: 1,
            rover_id: 3,
            run_id: "run".into(),
            sequence: 1,
            width: 2,
            height: 1,
            resolution: 0.5,
            origin_x: 0.,
            origin_y: 0.,
            occupancy: vec![0, 65],
        };
        let marks = fleet_marks(
            &snapshot,
            &localizations,
            &Value::Null,
            |id| (id == 3).then(|| (grid.clone(), 0.4)),
            origin,
        );
        assert_eq!(marks.len(), 1);
        let (expected_lat, expected_lon) = Anchor {
            latitude: 38.8297,
            longitude: -77.3075,
        }
        .geographic(12., 4.)
        .unwrap();
        assert!((marks[0].latitude - expected_lat).abs() < 1e-6);
        assert!((marks[0].longitude - expected_lon).abs() < 1e-6);
        assert!((marks[0].heading - 270.).abs() < 1e-6);
        assert_eq!(marks[0].status, "online");
        assert_eq!(marks[0].occupancy, "2x1, 1 occupied, 0.4s");
    }

    #[test]
    fn camera_fits_when_the_rover_set_changes() {
        let origin = LocalFrame::origin(DEFAULT_START_LATITUDE, DEFAULT_START_LONGITUDE).unwrap();
        let mut first = pose(0., 0., 0.);
        first.rover_id = 1;
        let snapshot = Snapshot {
            link: "healthy".into(),
            fleet_age: Some(0.),
            rovers: vec![rover(1, "online", Some(first.clone()), Some(0.1))],
            notice: None,
        };
        let marks = fleet_marks(&snapshot, &Value::Null, &Value::Null, |_| None, origin);
        let previous = BTreeSet::new();
        let (ids, latitude, longitude) = framing_target(&previous, &marks).unwrap();
        assert_eq!(ids, BTreeSet::from([1]));
        assert!((latitude - marks[0].latitude).abs() < 1e-9);
        assert!((longitude - marks[0].longitude).abs() < 1e-9);
        assert!(framing_target(&ids, &marks).is_none());

        let mut moved = first.clone();
        moved.x = 30.;
        let moved_snapshot = Snapshot {
            rovers: vec![rover(1, "online", Some(moved), Some(3.))],
            ..snapshot.clone()
        };
        let moved_marks = fleet_marks(
            &moved_snapshot,
            &Value::Null,
            &Value::Null,
            |_| None,
            origin,
        );
        assert_eq!(moved_marks[0].status, "pose stale");
        assert!(moved_marks[0].latitude > marks[0].latitude);
        assert!(framing_target(&ids, &moved_marks).is_none());

        let mut second = pose(10., 0., 0.);
        second.rover_id = 2;
        let both = Snapshot {
            rovers: vec![
                rover(1, "absent", Some(first), Some(1.)),
                rover(2, "online", Some(second), Some(0.1)),
            ],
            ..snapshot
        };
        let both_marks = fleet_marks(&both, &Value::Null, &Value::Null, |_| None, origin);
        assert_eq!(both_marks[0].status, "offline");
        let (ids, latitude, _) = framing_target(&ids, &both_marks).unwrap();
        assert_eq!(ids, BTreeSet::from([1, 2]));
        let mean = (both_marks[0].latitude + both_marks[1].latitude) / 2.;
        assert!((latitude - mean).abs() < 1e-9);
        assert!(framing_target(&BTreeSet::new(), &[]).is_none());
    }
}
