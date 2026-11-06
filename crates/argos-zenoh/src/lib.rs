//! Zenoh fleet client, telemetry subscriptions, and operator commands.
//!
//! [`Client::connect`] installs subscribers and starts the held-input worker.
//! Every rover's pose and small status topics are subscribed on connect. The
//! heavy ones (occupancy grid, point cloud, depth frames) are subscribed only
//! for the rovers passed to [`Client::focus`], so an idle dashboard does not
//! pull every rover's maps over the link.
//! The client keeps bounded latest-state caches and checks freshness and authority
//! before issuing commands. `router` manages the local router; `session_log`
//! writes session evidence off the command path. [`Client::disconnect`] retires
//! the session and stops its worker rather than replaying input on reconnect.
//!
//! | File | What it holds |
//! | --- | --- |
//! | `lib.rs` | `Client`: connect, focus, snapshots, and every send |
//! | `subscriptions.rs` | Each subscription opened on connect, one function each |
//! | `teleop_worker.rs` | The 50 ms held-key worker that publishes teleop twists |
//! | `command_checks.rs` | What `operator_command` checks before it publishes |
//! | `session_log.rs` | The session log file and export |
//! | `point_capture.rs` | 3D point capture and the live point stream |
//! | `router.rs` | A local Zenoh router for tests and Zorvane |

pub mod attention;
mod command_checks;
mod content_heartbeat;
pub mod observability;
mod point_capture;
pub use point_capture::{CaptureStatus, LIVE_RATE_HZ, LIVE_STRIDE, LiveStatus};
pub mod router;
mod session_log;
mod subscriptions;
mod teleop_worker;
use argos_core::*;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use zenoh::Wait;
type SharedLocalizations =
    Arc<Mutex<std::collections::BTreeMap<u64, (serde_json::Value, Instant)>>>;
type SharedMaps = Arc<Mutex<std::collections::BTreeMap<u64, (OccupancyGrid, f64, Option<String>)>>>;
type SharedTelemetry =
    Arc<Mutex<std::collections::BTreeMap<(u64, String), (serde_json::Value, Instant)>>>;
type Handler = Arc<dyn Fn(zenoh::sample::Sample) + Send + Sync>;
/// Authority and run state, kept for every rover.
const AUTHORITY_TOPICS: [&str; 4] = [
    "autonomy/status",
    "goal/proposal",
    "mission/status",
    "experiment/status",
];
/// Small operator telemetry, kept for every rover.
const STATE_TOPICS: [&str; 7] = [
    "hardware/status",
    "objects",
    "autonomy/status",
    "pose/quality",
    "navigation/outdoor",
    "search/report",
    "search/status",
];
/// Visualization topics routed through the state handler, focused rover only.
/// `map/occupancy` and `camera/depth` have their own handlers.
const DETAIL_TOPICS: [&str; 3] = [
    "map/occupancy/heartbeat",
    "pointcloud",
    "pointcloud/heartbeat",
];
/// Handlers for the focused rover's heavy topics, declared on demand.
struct DetailHandlers {
    occupancy: Handler,
    depth: Handler,
    state: Handler,
}
fn listen(
    session: &zenoh::Session,
    key: String,
    handler: &Handler,
) -> Result<zenoh::pubsub::Subscriber<()>> {
    let handler = handler.clone();
    session
        .declare_subscriber(key)
        .callback(move |sample| handler(sample))
        .wait()
        .map_err(|e| Error(e.to_string()))
}
/// Held keys belong to one rover and the authority generation that admitted them.
#[derive(Default)]
struct InputState {
    rover_id: Option<u64>,
    keys: HeldInput,
    authority_generation: Option<(String, u64)>,
    /// Set by the dashboard. Absent keeps the legacy unit twist.
    guard: Option<TeleopGuard>,
    deadman: bool,
    lease_until: f64,
    last_step: f64,
}
/// Default folder for session logs: `ARGOS_SESSION_DIR`, else the temp folder.
/// Desktop apps pass a lasting folder to [`Client::connect_logged`] instead.
pub fn session_dir() -> std::path::PathBuf {
    std::env::var_os("ARGOS_SESSION_DIR")
        .filter(|dir| !dir.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// One native session; subscriptions retain only latest parsed telemetry.
pub struct Client {
    session: Arc<Mutex<Option<zenoh::Session>>>,
    subscribers: Mutex<Vec<zenoh::pubsub::Subscriber<()>>>,
    /// Focused rovers and their heavy-topic subscribers.
    detail: Mutex<std::collections::BTreeMap<u64, Vec<zenoh::pubsub::Subscriber<()>>>>,
    detail_handlers: DetailHandlers,
    cache: Arc<Mutex<FleetCache>>,
    maps: SharedMaps,
    localizations: Arc<Mutex<std::collections::BTreeMap<u64, (serde_json::Value, Instant)>>>,
    operator_states: SharedTelemetry,
    actions: Mutex<()>,
    topics: Topics,
    start: Instant,
    operator: Arc<Mutex<OperatorCache>>,
    held: Arc<Mutex<InputState>>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    recorder: Arc<session_log::RunRecorder>,
    log_path: std::path::PathBuf,
    session_id: String,
    ui_sequence: Mutex<InputOrder>,
    teleop_sequence: Arc<std::sync::atomic::AtomicU64>,
    tracking: SharedTracking,
    /// Latest point capture per rover; subscribed only while one runs.
    captures: point_capture::Captures,
    /// Live point stream per rover; subscribed only while one is wanted.
    lives: point_capture::Lives,
    /// Who is sending commands right now, when the dashboard says so; tags
    /// interventions whose tokens do not carry it (stops, goal cancels).
    command_source: Mutex<Option<&'static str>>,
    /// Open help episodes per rover, from `autonomy/status`.
    help: SharedHelp,
    /// Last level request per rover, to drop repeated clicks.
    last_level: Mutex<std::collections::BTreeMap<u64, (String, Instant)>>,
}
pub(crate) type SharedHelp = Arc<Mutex<attention::HelpTracker>>;
type SharedTracking = Arc<Mutex<std::collections::BTreeMap<u64, Option<bool>>>>;
impl Client {
    pub fn connect(endpoint: &str, prefix: &str) -> Result<Self> {
        Self::connect_authenticated(endpoint, prefix, None)
    }
    pub fn connect_authenticated(
        endpoint: &str,
        prefix: &str,
        credentials: Option<(&str, &str)>,
    ) -> Result<Self> {
        Self::connect_logged(endpoint, prefix, credentials, &session_dir())
    }
    /// [`Self::connect_authenticated`] that writes the session log into
    /// `log_dir` (created if needed) as `argos-session-<uuid>.jsonl`.
    pub fn connect_logged(
        endpoint: &str,
        prefix: &str,
        credentials: Option<(&str, &str)>,
        log_dir: &std::path::Path,
    ) -> Result<Self> {
        terra_observability::initialize_runtime("argos");
        let topics = Topics::new(prefix)?;
        let config = subscriptions::client_config(endpoint, credentials)?;
        let session = zenoh::open(config)
            .wait()
            .map_err(|e| Error(format!("could not connect: {e}")))?;
        let cache = Arc::new(Mutex::new(FleetCache::default()));
        let start = Instant::now();
        let fleet = subscriptions::subscribe_fleet(&session, &topics, &cache, start)?;
        let status = subscriptions::subscribe_goal_status(&session, &topics, &cache, start)?;
        let tracking: SharedTracking = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
        let depth = subscriptions::depth_handler(&topics, &cache, &tracking, start);
        let direct_pose =
            subscriptions::subscribe_pose(&session, &topics, &cache, &tracking, start)?;
        let localizations: SharedLocalizations =
            Arc::new(Mutex::new(std::collections::BTreeMap::new()));
        let maps: SharedMaps = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
        let occupancy = subscriptions::occupancy_handler(&topics, &localizations, &maps, start);
        let operator = Arc::new(Mutex::new(OperatorCache::default()));
        let held = Arc::new(Mutex::new(InputState::default()));
        let (session_id, log_path, recorder) = subscriptions::open_session_log(log_dir);
        let help: SharedHelp = Arc::new(Mutex::new(attention::HelpTracker::default()));
        let authority = subscriptions::authority_handler(
            &topics,
            &cache,
            &operator,
            &held,
            &recorder,
            &help,
            &session_id,
            start,
        );
        let localization =
            subscriptions::subscribe_localization(&session, &topics, &localizations, &maps)?;
        let operator_states: SharedTelemetry =
            Arc::new(Mutex::new(std::collections::BTreeMap::new()));
        let operator_feed = subscriptions::telemetry_handler(
            &topics,
            &operator_states,
            &maps,
            &recorder,
            &session_id,
            start,
        );
        let mut subscribers = vec![fleet, status, direct_pose, localization];
        for topic in AUTHORITY_TOPICS {
            subscribers.push(listen(
                &session,
                format!("{}/*/{topic}", topics.prefix),
                &authority,
            )?);
        }
        for topic in STATE_TOPICS {
            subscribers.push(listen(
                &session,
                format!("{}/*/{topic}", topics.prefix),
                &operator_feed,
            )?);
        }
        let session = Arc::new(Mutex::new(Some(session)));
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let teleop_sequence = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let worker = teleop_worker::spawn_teleop_worker(
            &session,
            &held,
            &operator,
            &operator_states,
            &cache,
            &shutdown,
            &teleop_sequence,
            &topics,
            &session_id,
            &recorder,
            start,
        );
        Ok(Self {
            session,
            subscribers: Mutex::new(subscribers),
            detail: Mutex::new(std::collections::BTreeMap::new()),
            detail_handlers: DetailHandlers {
                occupancy,
                depth,
                state: operator_feed,
            },
            cache,
            maps,
            localizations,
            operator_states,
            actions: Mutex::new(()),
            last_level: Mutex::new(Default::default()),
            topics,
            start,
            operator,
            held,
            shutdown,
            worker: Mutex::new(Some(worker)),
            recorder,
            log_path,
            session_id,
            ui_sequence: Mutex::new(InputOrder::default()),
            teleop_sequence,
            tracking,
            captures: Default::default(),
            lives: Default::default(),
            command_source: Mutex::new(None),
            help,
        })
    }
    /// Subscribe to these rovers' occupancy grids, point clouds, and depth
    /// frames, and drop every other rover's. An empty set leaves the whole
    /// fleet on pose and status only; the dashboard passes its selection. A
    /// dropped rover's grid and cloud are cleared so a stale map is not drawn.
    /// Rovers already focused keep their subscriptions. Terra sends a full
    /// grid at least every two seconds, so a new focus fills quickly.
    pub fn focus(&self, ids: impl IntoIterator<Item = u64>) -> Result<()> {
        let wanted: std::collections::BTreeSet<u64> = ids.into_iter().collect();
        let mut detail = self.detail.lock().unwrap();
        let dropped: Vec<u64> = detail
            .keys()
            .filter(|id| !wanted.contains(id))
            .copied()
            .collect();
        for id in dropped {
            detail.remove(&id);
            self.maps.lock().unwrap().remove(&id);
            self.operator_states
                .lock()
                .unwrap()
                .remove(&(id, "cloud".to_string()));
        }
        let added: Vec<u64> = wanted
            .into_iter()
            .filter(|id| !detail.contains_key(id))
            .collect();
        if added.is_empty() {
            return Ok(());
        }
        let session = self.session.lock().unwrap();
        let session = session.as_ref().ok_or_else(|| invalid("disconnected"))?;
        let handlers = &self.detail_handlers;
        for id in added {
            let base = format!("{}/{id}", self.topics.prefix);
            let mut subscribers = vec![
                listen(
                    session,
                    format!("{base}/map/occupancy"),
                    &handlers.occupancy,
                )?,
                listen(session, format!("{base}/camera/depth"), &handlers.depth)?,
            ];
            for topic in DETAIL_TOPICS {
                subscribers.push(listen(session, format!("{base}/{topic}"), &handlers.state)?);
            }
            detail.insert(id, subscribers);
        }
        Ok(())
    }
    /// The rovers whose heavy topics are subscribed.
    pub fn focused(&self) -> Vec<u64> {
        self.detail.lock().unwrap().keys().copied().collect()
    }
    pub fn operator_state_snapshot(&self) -> String {
        let mut result = self.legacy_operator_snapshot_value();
        for ((id, kind), (value, received)) in self.operator_states.lock().unwrap().iter() {
            let age = received.elapsed().as_secs_f64();
            let max_age = if kind == "authority" || kind == "hardware" {
                SUPERVISION_STALE_SECONDS
            } else if kind == "outdoor" {
                3.0
            } else {
                2.5
            };
            if age >= max_age {
                continue;
            }
            let mut value = value.clone();
            value["age"] = serde_json::json!(age);
            if kind.starts_with("search_report:") {
                if !result[id.to_string()]["reports"].is_array() {
                    result[id.to_string()]["reports"] = serde_json::json!([]);
                }
                result[id.to_string()]["reports"]
                    .as_array_mut()
                    .unwrap()
                    .push(value);
            } else {
                result[id.to_string()][kind] = value;
            }
        }
        result.to_string()
    }
    pub fn pose_tracking(&self, id: u64) -> Option<bool> {
        self.tracking.lock().unwrap().get(&id).copied().flatten()
    }
    /// Replace the dashboard twist shaper. Legacy callers leave it unset.
    ///
    /// Defense in depth; Terra enforces the real limits.
    pub fn set_teleop_guard(&self, limits: TeleopLimits) {
        let mut held = self.held.lock().unwrap();
        if let Some(guard) = held.guard.as_mut() {
            guard.limits = limits;
        } else {
            held.guard = Some(TeleopGuard::new(limits));
        }
        held.deadman = true;
    }
    pub fn clear_teleop_guard(&self) {
        let mut held = self.held.lock().unwrap();
        if let Some(guard) = held.guard.as_mut() {
            guard.reset();
        }
        held.guard = None;
        held.deadman = false;
        held.lease_until = 0.;
    }
    pub fn refresh_drive(&self, id: u64) {
        let now = self.start.elapsed().as_secs_f64();
        let mut held = self.held.lock().unwrap();
        if held.deadman && held.rover_id == Some(id) && held.keys.is_held() {
            held.lease_until = now + DRIVE_DEADMAN_SECONDS;
        }
    }
    pub fn occupancy_grid(&self, id: u64) -> Option<(OccupancyGrid, f64, Option<String>)> {
        self.maps.lock().unwrap().get(&id).map(|(grid, at, frame)| {
            (
                grid.clone(),
                (self.start.elapsed().as_secs_f64() - at).max(0.),
                frame.clone(),
            )
        })
    }
    pub fn operator_snapshot(&self) -> String {
        let mut result: serde_json::Value = serde_json::from_str(&self.operator_state_snapshot())
            .unwrap_or_else(|_| serde_json::json!({}));
        if let serde_json::Value::Object(maps) = self.occupancy_snapshot_value() {
            for (id, mut entry) in maps {
                let mut grid = entry["grid"].take();
                grid["age"] = entry["age"].take();
                result[&id]["occupancy"] = grid;
            }
        }
        result.to_string()
    }
    /// Bind new remote work to the observed vehicle authority generation. No
    /// command is persisted/retried; link loss retires this generation onboard.
    fn bound_payload(&self, id: u64, kind: &str, payload: &[u8]) -> Result<Vec<u8>> {
        let mut v: serde_json::Value =
            serde_json::from_slice(payload).map_err(|_| invalid("invalid command JSON"))?;
        let neutral = (kind == "goal" && v["cancel"] == true)
            || (kind == "safety" && v["action"] == "stop")
            || (kind == "search/action"
                && matches!(v["action"].as_str(), Some("pause" | "cancel")))
            || kind == "search/report/ack"
            || kind == "mission/report"
            || (kind == "task/action" && v["action"] == "cancel")
            || (kind == "dock" && v["action"] == "cancel")
            || (kind == "teleop"
                && v["linear"].as_f64() == Some(0.)
                && v["angular"].as_f64() == Some(0.));
        if !neutral {
            let states = self.operator_states.lock().unwrap();
            let (authority, at) = states.get(&(id, "authority".into())).ok_or_else(|| {
                invalid("Current vehicle authority required; update or reconnect the vehicle")
            })?;
            if at.elapsed().as_secs_f64() >= SUPERVISION_STALE_SECONDS
                || !authority["run_id"].is_string()
                || authority["revision"].as_u64().is_none()
            {
                return Err(invalid("Current vehicle authority required"));
            }
            if (v.get("run_id").is_some() && v["run_id"] != authority["run_id"])
                || (v.get("authority_revision").is_some()
                    && v["authority_revision"] != authority["revision"])
            {
                return Err(invalid(
                    "Vehicle authority changed; review the command again",
                ));
            }
            v["run_id"] = authority["run_id"].clone();
            v["authority_revision"] = authority["revision"].clone();
        }
        serde_json::to_vec(&v).map_err(|_| invalid("invalid command JSON"))
    }
    pub fn operator_command(&self, id: u64, kind: &str, payload: &str) -> Result<()> {
        let _action = self.actions.lock().unwrap();
        if !matches!(
            kind,
            "autonomy"
                | "teleop"
                | "navigation/settings"
                | "navigation/source"
                | "navigation/resume"
                | "search"
                | "search/action"
                | "search/report/ack"
                | "task/action"
                | "dock"
        ) || payload.len() > 2048
        {
            return Err(invalid("invalid operator command"));
        }
        let value: serde_json::Value =
            serde_json::from_str(payload).map_err(|_| invalid("invalid command JSON"))?;
        command_checks::check_command_shape(kind, payload, &value)?;
        // Neutral may still be sent during telemetry loss; moving commands require fresh fleet and pose.
        let neutral = command_checks::is_neutral(kind, &value);
        let freshness = if kind == "autonomy"
            && matches!(
                value["level"].as_str(),
                Some("waypoint" | "waypoint_direct")
            ) {
            SUPERVISION_STALE_SECONDS
        } else {
            STALE_SECONDS
        };
        // L0 selection and confirmed L0 control do not consume camera pose.
        // Fresh membership, authority, run/revision and hardware checks below remain.
        // A study condition moves nothing: at most it drops the rover to L0.
        let manual_without_pose = (kind == "autonomy"
            && (value["level"] == "teleop" || value.get("assigned_level").is_some()))
            || (kind == "teleop"
                && self
                    .operator_states
                    .lock()
                    .unwrap()
                    .get(&(id, "authority".into()))
                    .is_some_and(|(authority, at)| {
                        at.elapsed().as_secs_f64() < STALE_SECONDS
                            && authority["requested_level"] == "teleop"
                    }));
        if !neutral {
            let snapshot = self.snapshot();
            if snapshot.link != "healthy"
                || !snapshot.rovers.iter().any(|r| {
                    r.id == id
                        && r.membership == "online"
                        && (manual_without_pose
                            || kind == "navigation/settings"
                            || r.pose_age.is_some_and(|a| a < freshness))
                })
            {
                return Err(invalid("fresh rover telemetry required"));
            }
        }
        if !neutral {
            self.check_vehicle_authority(id, kind, &value, freshness)?;
        }
        if kind == "autonomy"
            && let Some(level) = value["level"].as_str()
        {
            let authority = self
                .operator_states
                .lock()
                .unwrap()
                .get(&(id, "authority".into()))
                .filter(|(_, at)| at.elapsed().as_secs_f64() < STALE_SECONDS)
                .map(|(authority, _)| authority.clone());
            let last = self.last_level.lock().unwrap().get(&id).cloned();
            if let Some(reason) = argos_core::redundant_level_request(
                level,
                authority.as_ref(),
                last.as_ref()
                    .map(|(sent, at)| (sent.as_str(), at.elapsed().as_secs_f64())),
            ) {
                return Err(invalid(&reason));
            }
        }
        let payload = self.bound_payload(id, kind, payload.as_bytes())?;
        let intervention = self.intervention(id, kind, &value);
        let result = self
            .session
            .lock()
            .unwrap()
            .as_ref()
            .ok_or_else(|| invalid("disconnected"))?
            .put(format!("{}/{id}/{kind}", self.topics.prefix), payload)
            .wait()
            .map_err(|e| Error(e.to_string()));
        if result.is_ok() {
            if kind == "autonomy"
                && let Some(level) = value["level"].as_str()
            {
                self.last_level
                    .lock()
                    .unwrap()
                    .insert(id, (level.to_string(), Instant::now()));
            }
            let _ = self.recorder.record(serde_json::json!({
                "kind": if kind == "teleop" {"teleop_sent"} else {"action_sent"},
                "topic": kind,
                "rover_id": id,
                "token": value["token"],
                "help_id": self.open_help(id),
                "time": self.start.elapsed().as_secs_f64(),
                "utc": utc(),
                "session_id": self.session_id,
                "payload": value,
            }));
            self.record_intervention(id, intervention, &value["token"]);
        }
        result
    }
    /// Classify an operator command as an intervention on autonomy, using the
    /// rover's last reported level before the command takes effect.
    fn intervention(
        &self,
        id: u64,
        kind: &str,
        value: &serde_json::Value,
    ) -> Option<(&'static str, serde_json::Value)> {
        let from = self
            .operator_states
            .lock()
            .unwrap()
            .get(&(id, "authority".into()))
            .map(|(authority, _)| authority["effective_level"].clone())
            .unwrap_or(serde_json::Value::Null);
        let cause = intervention_cause(kind, value, &from)?;
        Some((cause, from))
    }
    fn record_intervention(
        &self,
        id: u64,
        intervention: Option<(&'static str, serde_json::Value)>,
        token: &serde_json::Value,
    ) {
        let Some((cause, from)) = intervention else {
            return;
        };
        let _ = self.recorder.record(serde_json::json!({
            "kind": "intervention",
            "cause": cause,
            "rover_id": id,
            // Prompted: the rover was waiting for help when the operator stepped in.
            "help_id": self.open_help(id),
            "prompted": self.open_help(id).is_some(),
            "from_level": from,
            "token": token,
            "source": (*self.command_source.lock().unwrap())
                .or_else(|| token.as_str().map(argos_core::token_source))
                .unwrap_or("operator"),
            "time": self.start.elapsed().as_secs_f64(),
            "utc": utc(),
            "session_id": self.session_id,
        }));
    }
    /// The help episode open for this rover, if it is waiting for the operator.
    pub fn open_help(&self, id: u64) -> Option<String> {
        self.help.lock().unwrap().open(id)
    }
    /// Mark the commands sent until the next call as coming from `source`
    /// (`agent`, `recommendation`), or `None` for the operator's own.
    pub fn set_command_source(&self, source: Option<&'static str>) {
        *self.command_source.lock().unwrap() = source;
    }
    /// Add a station event, such as an agent request or a recommendation
    /// decision, to the session log with the usual time stamps.
    pub fn record_event(&self, mut event: serde_json::Value) {
        if !event.is_object() {
            return;
        }
        event["time"] = self.start.elapsed().as_secs_f64().into();
        event["utc"] = utc().into();
        event["session_id"] = self.session_id.clone().into();
        let _ = self.recorder.record(event);
    }
    pub fn localization_snapshot(&self) -> String {
        let values: serde_json::Map<String, serde_json::Value> = self
            .localizations
            .lock()
            .unwrap()
            .iter()
            .map(|(id, (value, received))| {
                let age = received.elapsed().as_secs_f64();
                // Keep coordinate-frame identity and the original observation age.
                // Expiry here previously made old local AR data look geographically aligned.
                let mut value = value.clone();
                value["age"] = serde_json::json!(age);
                (id.to_string(), value)
            })
            .collect();
        serde_json::Value::Object(values).to_string()
    }
    pub fn occupancy_snapshot(&self) -> String {
        self.occupancy_snapshot_value().to_string()
    }
    fn occupancy_snapshot_value(&self) -> serde_json::Value {
        let now = self.start.elapsed().as_secs_f64();
        let fleet = self.cache.lock().unwrap().snapshot(now);
        // Copy a coherent set of typed grids under the lock; expanding cells into
        // JSON values and serializing the public wire payload happen afterwards.
        let grids: Vec<_> = {
            let maps = self.maps.lock().unwrap();
            fleet
                .rovers
                .iter()
                .filter(|r| r.membership != "absent")
                .filter_map(|rover| {
                    maps.get(&rover.id)
                        .map(|(grid, time, _)| (rover.id, grid.clone(), (now - time).max(0.)))
                })
                .collect()
        };
        let mut result = serde_json::Map::new();
        for (id, grid, age) in grids {
            result.insert(id.to_string(), serde_json::json!({"grid": grid,"age": age}));
        }
        serde_json::Value::Object(result)
    }
    fn legacy_operator_snapshot_value(&self) -> serde_json::Value {
        let mut state = self
            .operator
            .lock()
            .unwrap()
            .snapshot(self.start.elapsed().as_secs_f64());
        if let Some(entries) = state.as_object_mut() {
            for entry in entries.values_mut() {
                entry["recording"] = if self.recorder.failed() {
                    "incomplete".into()
                } else {
                    "active".into()
                };
            }
        }
        state
    }
    pub fn action(&self, id: u64, kind: &str, value: serde_json::Value) -> Result<String> {
        let token = uuid::Uuid::new_v4().to_string();
        let intervention = self.intervention(id, kind, &value);
        let payload = operator_payload(kind, value, &token)?;
        let payload = self.bound_payload(id, kind, &payload)?;
        if kind == "autonomy" || kind == "safety" {
            self.clear_input();
        }
        let now = self.start.elapsed().as_secs_f64();
        if kind != "mission/report" {
            self.operator
                .lock()
                .unwrap()
                .pending(id, token.clone(), now);
        }
        let session = self.session.lock().unwrap();
        let s = session.as_ref().ok_or_else(|| invalid("disconnected"))?;
        s.put(
            format!("{}/{id}/{kind}", self.topics.prefix),
            payload.clone(),
        )
        .wait()
        .map_err(|e| Error(e.to_string()))?;
        let _ = self.recorder.record(serde_json::json!({
            "kind": "action_sent",
            "rover_id": id,
            "token": token,
            "topic": kind,
            "help_id": self.open_help(id),
            "time": now,
            "utc": utc(),
            "session_id": self.session_id,
            "payload": serde_json::from_slice::<serde_json::Value>(&payload).unwrap(),
        }));
        self.record_intervention(id, intervention, &token.clone().into());
        Ok(token)
    }
    pub fn fleet_action(&self, kind: &str, value: serde_json::Value) -> Vec<(u64, String)> {
        let parent = uuid::Uuid::new_v4().to_string();
        let targets = self
            .snapshot()
            .rovers
            .into_iter()
            .map(|r| r.id)
            .collect::<Vec<_>>();
        let _ = self.recorder.record(serde_json::json!({
            "kind": "fleet_action",
            "parent_action_id": parent,
            "targets": targets,
            "time": self.start.elapsed().as_secs_f64(),
            "utc": utc(),
        }));
        targets
            .into_iter()
            .map(|id| {
                (
                    id,
                    self.action(id, kind, value.clone())
                        .unwrap_or_else(|e| format!("failed:{e}")),
                )
            })
            .collect()
    }
    pub fn input_generation(
        &self,
        id: u64,
        key: &str,
        down: bool,
        sequence: u64,
        run: &str,
        revision: u64,
    ) -> Result<()> {
        let mut order = self.ui_sequence.lock().unwrap();
        if !order.accept(key, sequence) {
            return Ok(());
        }
        self.input_checked(id, key, down, Some((run, revision)))
    }
    pub fn input_event(&self, id: u64, key: &str, down: bool, sequence: u64) -> Result<()> {
        let mut last = self.ui_sequence.lock().unwrap();
        if !last.accept(key, sequence) {
            return Ok(());
        }
        self.input(id, key, down)
    }
    pub fn clear_input_event(&self, sequence: u64) {
        let mut last = self.ui_sequence.lock().unwrap();
        if !last.clear(sequence) {
            return;
        }
        self.clear_input();
    }
    pub fn input(&self, id: u64, key: &str, down: bool) -> Result<()> {
        self.input_checked(id, key, down, None)
    }
    fn input_checked(
        &self,
        id: u64,
        key: &str,
        down: bool,
        generation: Option<(&str, u64)>,
    ) -> Result<()> {
        let now = self.start.elapsed().as_secs_f64();
        let mut held = self.held.lock().unwrap();
        if down {
            let states = self.operator_states.lock().unwrap();
            let (hardware, at) = states
                .get(&(id, "hardware".into()))
                .ok_or_else(|| invalid("Hardware status unavailable"))?;
            require_relay(hardware, at, false)?;
        }
        let accepted_generation = {
            let authority = self.operator.lock().unwrap();
            if generation
                .is_some_and(|(run, revision)| !authority.matches_authority(id, run, revision))
            {
                return Ok(());
            }
            if down && let Some(reason) = authority.drive_refusal(id, now) {
                return Err(invalid(&format!("cannot drive: {reason}")));
            }
            authority.generation(id)
        };
        if held.rover_id != Some(id) {
            if let Some(previous) = held.rover_id
                && let Some(s) = self.session.lock().unwrap().as_ref()
            {
                let _ = s
                    .put(
                        format!("{}/{previous}/teleop", self.topics.prefix),
                        zero_payload(
                            &self.session_id,
                            &self.teleop_sequence,
                            held.authority_generation.as_ref(),
                        ),
                    )
                    .wait();
            }
            held.keys.clear();
        }
        held.rover_id = Some(id);
        held.authority_generation = accepted_generation;
        if down {
            held.keys.press(key);
            if held.deadman {
                held.lease_until = now + DRIVE_DEADMAN_SECONDS;
            }
        } else {
            held.keys.release(key);
        }
        Ok(())
    }
    pub fn clear_input(&self) {
        let mut held = self.held.lock().unwrap();
        let id = held.rover_id.take();
        held.keys.clear();
        if let Some(id) = id
            && let Some(s) = self.session.lock().unwrap().as_ref()
        {
            let _ = s
                .put(
                    format!("{}/{id}/teleop", self.topics.prefix),
                    zero_payload(
                        &self.session_id,
                        &self.teleop_sequence,
                        held.authority_generation.as_ref(),
                    ),
                )
                .wait();
        }
    }
    /// The live session log this client appends to.
    pub fn session_log_path(&self) -> &std::path::Path {
        &self.log_path
    }
    /// Flush and export recorded session events as complete JSONL records.
    /// This exports the event log, not the live occupancy or point-cloud caches.
    pub fn export_session(&self, destination: &str) -> Result<()> {
        let log = &self.recorder;
        log.flush().map_err(|e| Error(e.to_string()))?;
        let _ = log.record(serde_json::json!({
            "kind": "session_snapshot",
            "recording_failed": log.failed(),
            "time": self.start.elapsed().as_secs_f64(),
            "utc": utc(),
        }));
        log.flush().map_err(|e| Error(e.to_string()))?;
        let bytes = std::fs::read(&self.log_path).map_err(|e| Error(e.to_string()))?;
        {
            let records = session_log::read_records(&bytes).map_err(|e| Error(e.to_string()))?;
            let mut export = Vec::new();
            for r in records {
                serde_json::to_writer(&mut export, &r).map_err(|e| Error(e.to_string()))?;
                export.push(b'\n');
            }
            std::fs::write(destination, export)
        }
        .map_err(|e| Error(e.to_string()))
    }
    pub fn snapshot(&self) -> Snapshot {
        self.cache
            .lock()
            .unwrap()
            .snapshot(self.start.elapsed().as_secs_f64())
    }
    pub fn send_goal(&self, id: u64, goal: GoalRequest) -> Result<String> {
        self.send_reviewed_goal(id, goal, None)
    }
    /// Preserve the geographic review envelope and recheck it immediately before publication.
    pub fn send_reviewed_goal(
        &self,
        id: u64,
        goal: GoalRequest,
        review: Option<(String, u64, f64)>,
    ) -> Result<String> {
        self.send_goal_attempt(id, goal, review, false)
    }
    /// Resend, once each, local goals Terra refused only because their
    /// authority revision was one behind (see `FleetCache::goal_refused`).
    /// Called from the fleet poll; the new send carries the current revision.
    pub fn resend_refused_goals(&self) -> Vec<(u64, Result<String>)> {
        let retries = self.cache.lock().unwrap().take_retries();
        retries
            .into_iter()
            .map(|(id, goal)| (id, self.send_goal_attempt(id, goal, None, true)))
            .collect()
    }
    fn send_goal_attempt(
        &self,
        id: u64,
        goal: GoalRequest,
        review: Option<(String, u64, f64)>,
        retried: bool,
    ) -> Result<String> {
        let _action = self.actions.lock().unwrap();
        if let GoalRequest::Geographic {
            latitude,
            longitude,
            ..
        } = goal
        {
            let (run, revision, radius) = review
                .as_ref()
                .ok_or_else(|| invalid("Geographic review required"))?;
            let states = self.operator_states.lock().unwrap();
            let (raw, at) = states
                .get(&(id, "outdoor".into()))
                .ok_or_else(|| invalid("Outdoor status unavailable"))?;
            let mut report = argos_core::OutdoorReport::decode(raw)?;
            report.age_s = Some(at.elapsed().as_secs_f64());
            let (authority, at) = states
                .get(&(id, "authority".into()))
                .ok_or_else(|| invalid("Authority unavailable"))?;
            if at.elapsed().as_secs_f64() >= SUPERVISION_STALE_SECONDS
                || authority["run_id"] != *run
                || authority["safety"] != "clear"
            {
                return Err(invalid("Authority changed"));
            }
            argos_core::admit_outdoor_goal(argos_core::OutdoorAdmission {
                report: Some(&report),
                latitude,
                longitude,
                arrival_radius_m: *radius,
                level: authority["requested_level"].as_str(),
                run_id: run,
                navigation_revision: *revision,
            })
            .map_err(invalid)?;
            let (hardware, at) = states
                .get(&(id, "hardware".into()))
                .ok_or_else(|| invalid("Hardware unavailable"))?;
            require_relay(hardware, at, false)?;
        }
        let token = uuid::Uuid::new_v4().to_string();
        let mut payload = self.cache.lock().unwrap().prepare_goal_attempt(
            id,
            &goal,
            &token,
            self.start.elapsed().as_secs_f64(),
            retried,
        )?;
        if let Some((run, revision, radius)) = review {
            let mut value: serde_json::Value =
                serde_json::from_slice(&payload).map_err(|_| invalid("Invalid goal"))?;
            value["run_id"] = serde_json::json!(run);
            value["navigation_revision"] = serde_json::json!(revision);
            value["arrival_radius_m"] = serde_json::json!(radius);
            payload = serde_json::to_vec(&value).map_err(|_| invalid("Invalid goal"))?;
        }
        payload = self
            .bound_payload(id, "goal", &payload)
            .inspect_err(|error| {
                self.cache
                    .lock()
                    .unwrap()
                    .publish_failed(id, error.to_string());
            })?;
        self.publish(id, payload.clone())?;
        let _ = self.recorder.record(serde_json::json!({
            "kind": "goal_sent",
            "rover_id": id,
            "token": token,
            "retry": retried,
            "help_id": self.open_help(id),
            "time": self.start.elapsed().as_secs_f64(),
            "utc": utc(),
            "session_id": self.session_id,
            "payload": serde_json::from_slice::<serde_json::Value>(&payload).ok(),
        }));
        Ok(token)
    }
    pub fn cancel_goal(&self, id: u64) -> Result<()> {
        let _action = self.actions.lock().unwrap();
        self.cache
            .lock()
            .unwrap()
            .prepare_cancel(id, self.start.elapsed().as_secs_f64())?;
        let _ = self.recorder.record(serde_json::json!({
            "kind": "goal_cancel_sent",
            "rover_id": id,
            "help_id": self.open_help(id),
            "time": self.start.elapsed().as_secs_f64(),
            "utc": utc(),
            "session_id": self.session_id,
        }));
        let intervention = self.intervention(id, "goal/cancel", &serde_json::Value::Null);
        let result = self.publish(id, cancel_payload().to_vec());
        if result.is_ok() {
            self.record_intervention(id, intervention, &serde_json::Value::Null);
        }
        result
    }
    fn publish(&self, id: u64, payload: Vec<u8>) -> Result<()> {
        let session = self.session.lock().unwrap();
        let result = session
            .as_ref()
            .ok_or_else(|| invalid("disconnected"))
            .and_then(|s| {
                s.put(self.topics.goal(id), payload)
                    .wait()
                    .map_err(|e| Error(e.to_string()))
            });
        if let Err(e) = &result {
            self.cache.lock().unwrap().publish_failed(id, e.to_string());
        }
        result
    }
    pub fn disconnect(&self) {
        terra_observability::record_runtime(
            "_producer",
            "router",
            "connection.disconnect",
            serde_json::json!({}),
        );
        let _action = self.actions.lock().unwrap();
        self.clear_input();
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(w) = self.worker.lock().unwrap().take() {
            let _ = w.join();
        }
        self.subscribers.lock().unwrap().clear();
        self.detail.lock().unwrap().clear();
        self.captures.lock().unwrap().clear();
        self.lives.lock().unwrap().clear();
        let _ = self.recorder.close();
        if let Some(session) = self.session.lock().unwrap().take() {
            let _ = session.close().wait();
        }
        self.cache.lock().unwrap().disconnected();
        self.tracking.lock().unwrap().clear();
        self.localizations.lock().unwrap().clear();
        self.operator_states.lock().unwrap().clear();
        self.maps.lock().unwrap().clear();
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.disconnect();
    }
}

fn require_relay(value: &serde_json::Value, at: &Instant, neutral: bool) -> Result<()> {
    let mut status = HardwareObservation::from_status(value);
    status.age = at.elapsed().as_secs_f64();
    if let Some(reason) = drive_block(&status) {
        return Err(invalid(reason));
    }
    if neutral && status.output_active {
        return Err(invalid("Navigation changes require neutral motor output"));
    }
    Ok(())
}

fn utc() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn zero_payload(
    session: &str,
    sequence: &std::sync::atomic::AtomicU64,
    generation: Option<&(String, u64)>,
) -> String {
    let sequence = sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    serde_json::json!({
        "linear": 0,
        "angular": 0,
        "operator_session_id": session,
        "sequence": sequence,
        "run_id": generation.map(|g|&g.0),
        "authority_revision": generation.map(|g|g.1),
    })
    .to_string()
}

#[cfg(test)]
#[path = "../../../tests/argos-zenoh/unit/lib_forest_tests.rs"]
mod forest_tests;

/// Why an operator command counts as an intervention on autonomy, if it does.
/// `from` is the rover's effective level before the command.
fn intervention_cause(
    kind: &str,
    value: &serde_json::Value,
    from: &serde_json::Value,
) -> Option<&'static str> {
    let autonomous = !matches!(from.as_str(), None | Some("teleop" | "assisted_teleop"));
    match kind {
        "safety" if value["action"] == "stop" => Some("stop"),
        "autonomy"
            if autonomous
                && value.get("assigned_level").is_none()
                && matches!(value["level"].as_str(), Some("teleop" | "assisted_teleop")) =>
        {
            Some("takeover")
        }
        "goal/cancel" if autonomous => Some("goal_cancel"),
        "search/action" | "task/action"
            if matches!(value["action"].as_str(), Some("cancel" | "pause")) =>
        {
            Some("mission_cancel")
        }
        "dock" if value["action"] == "cancel" => Some("dock_cancel"),
        _ => None,
    }
}

#[cfg(test)]
mod intervention_tests {
    use super::intervention_cause;
    use serde_json::json;

    #[test]
    fn classifies_operator_interventions() {
        let auto = json!("waypoint");
        let manual = json!("teleop");
        let to_l0 = json!({"level": "teleop"});
        assert_eq!(
            intervention_cause("autonomy", &to_l0, &auto),
            Some("takeover")
        );
        assert_eq!(intervention_cause("autonomy", &to_l0, &manual), None);
        assert_eq!(intervention_cause("autonomy", &to_l0, &json!(null)), None);
        assert_eq!(
            intervention_cause("autonomy", &json!({"level": "explore"}), &auto),
            None
        );
        assert_eq!(
            intervention_cause("autonomy", &json!({"assigned_level": "teleop"}), &auto),
            None
        );
        assert_eq!(
            intervention_cause("safety", &json!({"action": "stop"}), &manual),
            Some("stop")
        );
        assert_eq!(
            intervention_cause("safety", &json!({"action": "reset"}), &auto),
            None
        );
        assert_eq!(
            intervention_cause("goal/cancel", &json!(null), &auto),
            Some("goal_cancel")
        );
        assert_eq!(
            intervention_cause("task/action", &json!({"action": "cancel"}), &json!("task")),
            Some("mission_cancel")
        );
    }
}
