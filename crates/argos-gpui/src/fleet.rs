//! Background Zenoh subscription and the marker picture the map draws.
//!
//! Connect and snapshot polling run on a thread named `argos-fleet`. The UI
//! only reads [`FleetPicture`].

use argos_core::{
    LocalFrame, OccupancyGrid, Snapshot, Topics, map_heading_degrees, rover_geographic,
};
use argos_zenoh::Client;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// GMU Fairfax. Used only when the start-location env vars are unset.
pub const DEFAULT_START_LATITUDE: f64 = 38.8297;
pub const DEFAULT_START_LONGITUDE: f64 = -77.3075;
const POSE_FRESH_SECONDS: f64 = 2.5;

#[derive(Clone, Debug)]
pub struct DashboardConfig {
    pub endpoint: String,
    pub prefix: String,
    pub credentials: Option<(String, String)>,
    pub start_latitude: f64,
    pub start_longitude: f64,
    pub origin: LocalFrame,
}
impl DashboardConfig {
    pub fn from_env() -> Result<Self, String> {
        let var = |key: &str| std::env::var(key).ok();
        Self::parse(
            var("ARGOS_ZENOH_ENDPOINT").as_deref(),
            var("ARGOS_ZENOH_PREFIX").as_deref(),
            var("ARGOS_ZENOH_USER").as_deref(),
            var("ARGOS_ZENOH_PASSWORD").as_deref(),
            (
                var("ARGOS_MAP_START_LAT").as_deref(),
                var("ARGOS_MAP_START_LON").as_deref(),
            ),
            (
                var("ARGOS_MAP_ORIGIN_LAT").as_deref(),
                var("ARGOS_MAP_ORIGIN_LON").as_deref(),
            ),
        )
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
            None => "tcp/127.0.0.1:7447".into(),
        };
        if endpoint.len() > 512 || !(endpoint.starts_with("tcp/") || endpoint.starts_with("tls/")) {
            return Err("ARGOS_ZENOH_ENDPOINT must be tcp/host:port or tls/host:port".into());
        }
        let prefix = prefix.map(str::trim).filter(|s| !s.is_empty());
        let prefix = Topics::new(prefix.unwrap_or("terra/rover")).map_err(|e| e.to_string())?;
        let user = user.filter(|s| !s.is_empty());
        let password = password.filter(|s| !s.is_empty());
        let credentials = match (user, password) {
            (None, None) => None,
            (Some(user), Some(password)) => Some((user.to_string(), password.to_string())),
            _ => {
                return Err(
                    "ARGOS_ZENOH_USER and ARGOS_ZENOH_PASSWORD must both be set, or neither".into(),
                );
            }
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
            start_latitude,
            start_longitude,
            origin,
        })
    }
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
pub struct RoverMark {
    pub id: u64,
    pub latitude: f64,
    pub longitude: f64,
    pub heading: f64,
    /// `online`, `pose stale`, or `offline`.
    pub status: String,
    pub occupancy: String,
}
#[derive(Clone, Debug)]
pub struct FleetPicture {
    pub link: String,
    pub detail: String,
    pub marks: Vec<RoverMark>,
}
impl Default for FleetPicture {
    fn default() -> Self {
        Self {
            link: "connecting".into(),
            detail: "Connecting".into(),
            marks: Vec::new(),
        }
    }
}

pub fn fleet_marks(
    snapshot: &Snapshot,
    localizations: &Value,
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
            Some(RoverMark {
                id: rover.id,
                latitude,
                longitude,
                heading: map_heading_degrees(pose.yaw),
                status: status.into(),
                occupancy: occupancy(rover.id)
                    .map(|(grid, age)| occupancy_summary(&grid, age))
                    .unwrap_or_else(|| "no occupancy".into()),
            })
        })
        .collect()
}
fn occupancy_summary(grid: &OccupancyGrid, age: f64) -> String {
    let occupied = grid.occupancy.iter().filter(|cell| **cell > 0).count();
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
}
impl FleetFeed {
    pub fn start(config: DashboardConfig) -> Self {
        let picture = Arc::new(Mutex::new(FleetPicture {
            link: "connecting".into(),
            detail: format!("Connecting to {} ({})", config.endpoint, config.prefix),
            marks: Vec::new(),
        }));
        let shared = picture.clone();
        std::thread::Builder::new()
            .name("argos-fleet".into())
            .spawn(move || run(config, shared))
            .expect("argos-fleet thread");
        Self { picture }
    }
    pub fn picture(&self) -> FleetPicture {
        self.picture.lock().unwrap().clone()
    }
}
fn run(config: DashboardConfig, picture: Arc<Mutex<FleetPicture>>) {
    loop {
        let credentials = config
            .credentials
            .as_ref()
            .map(|(user, password)| (user.as_str(), password.as_str()));
        match Client::connect_authenticated(&config.endpoint, &config.prefix, credentials) {
            Ok(client) => poll(&client, &config, &picture),
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
    loop {
        let snapshot = client.snapshot();
        if snapshot.link == "failed" || snapshot.link == "disconnected" {
            break;
        }
        let localizations =
            serde_json::from_str::<Value>(&client.localization_snapshot()).unwrap_or(Value::Null);
        let marks = fleet_marks(
            &snapshot,
            &localizations,
            |id| client.occupancy_grid(id).map(|(grid, age, _)| (grid, age)),
            config.origin,
        );
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
    fn unset_env_defaults_to_the_gmu_fallback_and_zorvane_port() {
        let config =
            DashboardConfig::parse(None, None, None, None, (None, None), (None, None)).unwrap();
        assert_eq!(config.endpoint, "tcp/127.0.0.1:7447");
        assert_eq!(config.prefix, "terra/rover");
        assert!(config.credentials.is_none());
        assert_eq!(config.start_latitude, DEFAULT_START_LATITUDE);
        assert_eq!(config.start_longitude, DEFAULT_START_LONGITUDE);
        assert_eq!(config.origin.latitude, DEFAULT_START_LATITUDE);
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
            DashboardConfig::parse(None, None, Some("terra"), None, (None, None), (None, None))
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
            occupancy: vec![0, 40],
        };
        let marks = fleet_marks(
            &snapshot,
            &localizations,
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
        let marks = fleet_marks(&snapshot, &Value::Null, |_| None, origin);
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
        let moved_marks = fleet_marks(&moved_snapshot, &Value::Null, |_| None, origin);
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
        let both_marks = fleet_marks(&both, &Value::Null, |_| None, origin);
        assert_eq!(both_marks[0].status, "offline");
        let (ids, latitude, _) = framing_target(&ids, &both_marks).unwrap();
        assert_eq!(ids, BTreeSet::from([1, 2]));
        let mean = (both_marks[0].latitude + both_marks[1].latitude) / 2.;
        assert!((latitude - mean).abs() < 1e-9);
        assert!(framing_target(&BTreeSet::new(), &[]).is_none());
    }
}
