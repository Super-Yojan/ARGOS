//! Dashboard settings: built-in defaults, `.env` loading, environment
//! variables, and where session logs and captures are saved.

use argos_core::{DegradeLimits, LocalFrame, TeleopLimits, Topics};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// GMU Fairfax. Used only when the start-location env vars are unset.
pub const DEFAULT_START_LATITUDE: f64 = 38.8297;

pub const DEFAULT_START_LONGITUDE: f64 = -77.3075;

pub const DEFAULT_ENDPOINT: &str = "tls/argos.34.135.204.210.sslip.io:7448";

pub const DEFAULT_PREFIX: &str = "terra/phone";

pub const DEFAULT_USER: &str = "terra";

pub const PASSWORD_NOTICE: &str = "Set ARGOS_ZENOH_PASSWORD in .env";

pub const MAPS_API_KEY_ENV: &str = "GOOGLE_MAPS_API_KEY";

pub const MAPS_KEY_NOTICE: &str = "Set GOOGLE_MAPS_API_KEY in .env";

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

/// Where session logs go: `ARGOS_SESSION_DIR`, else `Documents/ARGOS Sessions`
/// in the home folder, else the temp folder (which the OS may clear).
pub fn session_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ARGOS_SESSION_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join("Documents").join("ARGOS Sessions"))
        .unwrap_or_else(std::env::temp_dir)
}

/// Where Capture 3D saves PLY files: `ARGOS_CAPTURE_DIR`, else
/// `Documents/ARGOS Captures` in the home folder, else the temp folder.
pub fn capture_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ARGOS_CAPTURE_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join("Documents").join("ARGOS Captures"))
        .unwrap_or_else(std::env::temp_dir)
}
