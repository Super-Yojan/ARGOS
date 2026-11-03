//! Turn the Zenoh snapshot into what the map draws: one `RoverMark` per
//! rover, fleet presence rows, and the camera framing target.

use argos_core::{
    CellClass, CommandOrigin, ExplorationReport, GoalState, HardwareObservation, LocalFrame,
    LocalMap, OccupancyGrid, OutdoorReport, Pose, RawGps, RouteSample, Snapshot, classify_cell,
    decode_autonomy_value, decode_exploration, decode_raw_gps, local_map, map_heading_degrees,
    reported_outdoor, reported_pose_quality,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const POSE_FRESH_SECONDS: f64 = 2.5;

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
    /// Whether the rover still sends pose or hardware status. The router
    /// link can look healthy while the phone has stopped publishing.
    pub reporting: argos_core::Reporting,
    pub occupancy: String,
    pub grid: Option<LocalMap>,
    pub requested_level: Option<String>,
    pub effective_level: Option<String>,
    pub assigned_level: Option<String>,
    pub route: Vec<RouteSample>,
    pub goal: Option<GoalMark>,
    pub degraded: bool,
    pub degrade_reason: String,
    /// True when a pose/quality report, not the yaw-rate fallback, set `degraded`.
    pub quality_reported: bool,
    pub reversing: bool,
    pub hold: Option<String>,
    /// Terra's reason for the current decision; explains a hold when no level is in effect.
    pub autonomy_reason: Option<String>,
    pub sensor_hold: Option<argos_core::SensorHold>,
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
    /// Terra's L5 task status once a task has started.
    pub task: Option<argos_core::TaskReport>,
    /// The phone advertises `task`, so it can run a language model.
    pub task_supported: bool,
    /// Who can answer an L5 task on this phone: the built-in model, an MCP agent, or both.
    pub task_agents: Vec<argos_core::TaskAgent>,
    /// Buckets and balls the rover's detector has confirmed this run.
    pub objects: Vec<ObjectMark>,
    /// Terra's bucket dock, once one has started this run.
    pub dock: Option<argos_core::DockReport>,
}

/// A found object. Its local map position is the truth: the dock and the
/// planner use it. The map draws it through the same frame as the rover, so
/// it lands where the rover saw it; the rover's GPS lat/lon (meters off
/// outdoors) is only extra info on `object`.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectMark {
    pub object: argos_core::FoundObject,
    /// Where the map draws it, from local x/y; `None` without a map frame.
    pub map_position: Option<(f64, f64)>,
}

fn object_marks(operator: &Value, overlay_frame: Option<LocalFrame>) -> Vec<ObjectMark> {
    argos_core::decode_objects(&operator["objects"])
        .into_iter()
        .map(|object| ObjectMark {
            map_position: overlay_frame.and_then(|frame| frame.geographic(object.x, object.y).ok()),
            object,
        })
        .collect()
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
    pub(super) id: u64,
    pub(super) membership: String,
    pose_available: bool,
    pub(super) observation_age: Option<f64>,
    pub(super) position_reason: String,
    pub(super) source_reason: String,
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

pub(super) fn scope_localization(mut report: Value, current_run: Option<&str>) -> Value {
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
pub(super) struct LocalizationHistory {
    values: BTreeMap<String, (Value, f64)>,
    membership: BTreeMap<u64, String>,
    retired_at: BTreeMap<u64, f64>,
}

impl LocalizationHistory {
    pub(super) fn observe_membership(&mut self, rows: &[(u64, &str)], now: f64) {
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
    pub(super) fn retain(&mut self, ids: &BTreeSet<u64>) {
        self.values
            .retain(|id, _| id.parse::<u64>().is_ok_and(|id| ids.contains(&id)));
    }
    pub(super) fn scope_runs(&mut self, operator: &Value) {
        for (id, (report, _)) in &mut self.values {
            *report =
                scope_localization(report.clone(), operator[id]["authority"]["run_id"].as_str());
        }
    }
    pub(super) fn update(&mut self, incoming: &Value, now: f64) -> Value {
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
                reporting: if status == "offline" {
                    argos_core::Reporting::Fresh
                } else {
                    argos_core::Reporting::classify(rover.pose_age, &hardware)
                },
                occupancy: grid
                    .as_ref()
                    .map(|(grid, age)| occupancy_summary(grid, *age))
                    .unwrap_or_else(|| "no occupancy".into()),
                grid: grid.as_ref().map(|(grid, _)| local_map(grid, 256)),
                requested_level: report.as_ref().map(|report| report.requested.clone()),
                effective_level: report.as_ref().and_then(|report| report.effective.clone()),
                assigned_level: report.as_ref().and_then(|report| report.assigned.clone()),
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
                autonomy_reason: report.as_ref().and_then(|report| report.reason.clone()),
                sensor_hold: report
                    .as_ref()
                    .and_then(|report| report.sensor_hold.clone()),
                command_origin: report
                    .as_ref()
                    .map(|report| report.origin)
                    .unwrap_or_default(),
                arrival_radius: outdoor
                    .as_ref()
                    .map(|report| report.arrival_radius_m)
                    .unwrap_or(argos_core::ARRIVAL_RADIUS_DEFAULT_M),
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
                task: report
                    .as_ref()
                    .and_then(|_| argos_core::decode_task(authority)),
                task_supported: authority["supported_levels"]
                    .as_array()
                    .is_some_and(|levels| levels.iter().any(|level| level == "task")),
                task_agents: argos_core::task_agents(
                    authority,
                    authority["supported_levels"]
                        .as_array()
                        .is_some_and(|levels| levels.iter().any(|level| level == "task")),
                ),
                objects: object_marks(&operator[&key], overlay_frame),
                dock: report.as_ref().and_then(|report| report.dock.clone()),
            })
        })
        .collect()
}

pub(super) fn display_policy(
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

pub(super) fn display_link(snapshot: &Snapshot, localizations: &Value, operator: &Value) -> String {
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

pub(super) fn retain_geographic_markers(
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
