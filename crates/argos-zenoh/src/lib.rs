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

mod content_heartbeat;
pub mod observability;
mod point_capture;
pub use point_capture::{CaptureStatus, LIVE_RATE_HZ, LIVE_STRIDE, LiveStatus};
pub mod router;
mod session_log;
use argos_core::*;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use zenoh::Wait;
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
const STATE_TOPICS: [&str; 6] = [
    "hardware/status",
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
}
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
        terra_observability::initialize_runtime("argos");
        let topics = Topics::new(prefix)?;
        if endpoint.len() > 512
            || !(endpoint.starts_with("tcp/") || endpoint.starts_with("tls/"))
            || endpoint.parse::<zenoh::config::EndPoint>().is_err()
        {
            return Err(invalid(
                "endpoint must be a valid tcp/host:port or tls/host:port",
            ));
        }
        let mut config = zenoh::Config::default();
        config
            .insert_json5("mode", "\"client\"")
            .map_err(|e| Error(e.to_string()))?;
        config
            .insert_json5(
                "connect/endpoints",
                &serde_json::to_string(&vec![endpoint]).unwrap(),
            )
            .map_err(|e| Error(e.to_string()))?;
        config
            .insert_json5("scouting/multicast/enabled", "false")
            .map_err(|e| Error(e.to_string()))?;
        config
            .insert_json5("connect/timeout_ms", "3000")
            .map_err(|e| Error(e.to_string()))?;
        if let Some((user, password)) = credentials {
            for (key, value) in [
                ("transport/auth/usrpwd/user", user),
                ("transport/auth/usrpwd/password", password),
            ] {
                config
                    .insert_json5(key, &serde_json::to_string(value).unwrap())
                    .map_err(|e| Error(e.to_string()))?;
            }
        }
        let session = zenoh::open(config)
            .wait()
            .map_err(|e| Error(format!("could not connect: {e}")))?;
        let cache = Arc::new(Mutex::new(FleetCache::default()));
        let start = Instant::now();
        let fleet_cache = cache.clone();
        let fleet = session
            .declare_subscriber(topics.fleet())
            .callback(move |sample| {
                let payload = sample.payload().to_bytes();
                let mut cache = fleet_cache.lock().unwrap();
                match decode_fleet(&payload) {
                    Ok(f) => cache.fleet(f, start.elapsed().as_secs_f64()),
                    Err(e) => cache.notice(e.to_string()),
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let fleet_cache = cache.clone();
        let subscribed_topics = topics.clone();
        let status = session
            .declare_subscriber(topics.statuses())
            .callback(move |sample| {
                let Some(id) =
                    subscribed_topics.rover_from_key(sample.key_expr().as_str(), "/goal/status")
                else {
                    return;
                };
                let payload = sample.payload().to_bytes();
                let mut cache = fleet_cache.lock().unwrap();
                match decode_status(&payload) {
                    Ok(s) => cache.status(id, s, start.elapsed().as_secs_f64()),
                    Err(e) => cache.notice(e.to_string()),
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let tracking: SharedTracking = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
        let fleet_cache = cache.clone();
        let subscribed_topics = topics.clone();
        let depth_tracking = tracking.clone();
        let depth: Handler = Arc::new(move |sample: zenoh::sample::Sample| {
            let Some(id) =
                subscribed_topics.rover_from_key(sample.key_expr().as_str(), "/camera/depth")
            else {
                return;
            };
            let payload = sample.payload().to_bytes();
            let flag = serde_json::from_slice::<serde_json::Value>(&payload)
                .ok()
                .as_ref()
                .and_then(pose_tracking_flag);
            let mut cache = fleet_cache.lock().unwrap();
            match decode_pose(&payload) {
                Ok(Some(p)) => {
                    depth_tracking.lock().unwrap().insert(id, flag);
                    cache.pose(id, p, start.elapsed().as_secs_f64());
                }
                Ok(None) => {}
                Err(e) => cache.notice(e.to_string()),
            }
        });
        let direct_cache = cache.clone();
        let direct_topics = topics.clone();
        let pose_tracking = tracking.clone();
        let direct_pose = session
            .declare_subscriber(format!("{}/*/pose", topics.prefix))
            .callback(move |sample| {
                let bytes = sample.payload().to_bytes();
                if let Some(id) = direct_topics.rover_from_key(sample.key_expr().as_str(), "/pose")
                    && let Ok(pose) = decode_topic_pose(&bytes, Some(id))
                {
                    let flag = serde_json::from_slice::<serde_json::Value>(&bytes)
                        .ok()
                        .as_ref()
                        .and_then(pose_tracking_flag);
                    pose_tracking.lock().unwrap().insert(id, flag);
                    direct_cache
                        .lock()
                        .unwrap()
                        .pose(id, pose, start.elapsed().as_secs_f64());
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let localizations = Arc::new(Mutex::new(std::collections::BTreeMap::<
            u64,
            (serde_json::Value, Instant),
        >::new()));
        let map_frames = localizations.clone();
        let maps = Arc::new(Mutex::new(std::collections::BTreeMap::<
            u64,
            (OccupancyGrid, f64, Option<String>),
        >::new()));
        let grids = maps.clone();
        let map_topics = topics.clone();
        let occupancy: Handler = Arc::new(move |sample: zenoh::sample::Sample| {
            let Some(id) = map_topics.rover_from_key(sample.key_expr().as_str(), "/map/occupancy")
            else {
                return;
            };
            let Ok(grid) = decode_occupancy(&sample.payload().to_bytes()) else {
                return;
            };
            if grid.rover_id != id {
                return;
            }
            let frames = map_frames.lock().unwrap();
            let frame_id = frames
                .get(&id)
                .and_then(|(v, _)| v["frameID"].as_str())
                .map(String::from);
            let mut maps = grids.lock().unwrap();
            if maps.get(&id).is_some_and(|(old, _, _)| {
                old.run_id == grid.run_id && old.sequence >= grid.sequence
            }) {
                return;
            }
            // Keep memory bounded even if an endpoint publishes unlisted IDs.
            if !maps.contains_key(&id) && maps.len() >= 256 {
                return;
            }
            maps.insert(id, (grid, start.elapsed().as_secs_f64(), frame_id));
        });
        let operator = Arc::new(Mutex::new(OperatorCache::default()));
        let held = Arc::new(Mutex::new(InputState::default()));
        let restart_input = held.clone();
        let session_id = uuid::Uuid::new_v4().to_string();
        let log_path = std::env::temp_dir().join(format!("argos-session-{session_id}.jsonl"));
        let recorder = Arc::new(
            session_log::RunRecorder::create(
                &log_path,
                serde_json::json!({
                    "session_id": session_id,
                    "utc": utc(),
                    "platform": "argos",
                    "clock": "session_relative_monotonic",
                }),
                8192,
            )
            .unwrap_or_else(|_| session_log::RunRecorder::unavailable()),
        );
        let authority_cache = operator.clone();
        let log = recorder.clone();
        let subscribed_topics = topics.clone();
        let fleet_cache = cache.clone();
        let callback_session_id = session_id.clone();
        let authority: Handler = Arc::new(move |sample: zenoh::sample::Sample| {
            for kind in AUTHORITY_TOPICS {
                let Some(id) = subscribed_topics
                    .rover_from_key(sample.key_expr().as_str(), &format!("/{kind}"))
                else {
                    continue;
                };
                if !fleet_cache
                    .lock()
                    .unwrap()
                    .snapshot(start.elapsed().as_secs_f64())
                    .rovers
                    .iter()
                    .any(|r| r.id == id && r.membership != "absent")
                {
                    return;
                }
                let bytes = sample.payload().to_bytes();
                let now = start.elapsed().as_secs_f64();
                let (ok, changed) = {
                    let mut state = authority_cache.lock().unwrap();
                    let previous = state.run_id(id);
                    let ok = state.apply(id, kind, &bytes, now).is_ok();
                    (ok, previous != state.run_id(id))
                };
                if changed {
                    let mut input = restart_input.lock().unwrap();
                    if input.rover_id == Some(id) {
                        input.rover_id = None;
                        input.keys.clear();
                        input.authority_generation = None;
                    }
                }
                if ok {
                    let _ = log.record(serde_json::json!({
                        "kind": "state_observed",
                        "topic": kind,
                        "rover_id": id,
                        "time": now,
                        "utc": utc(),
                        "session_id": callback_session_id,
                        "payload": serde_json::from_slice::<serde_json::Value>(&bytes).ok(),
                    }));
                }
                return;
            }
        });
        let l = localizations.clone();
        let frame_grids = maps.clone();
        let subscribed_topics = topics.clone();
        let localization = session
            .declare_subscriber(format!("{}/*/localization", topics.prefix))
            .callback(move |sample| {
                let Some(id) =
                    subscribed_topics.rover_from_key(sample.key_expr().as_str(), "/localization")
                else {
                    return;
                };
                let bytes = sample.payload().to_bytes();
                if bytes.len() > 4096 {
                    return;
                }
                let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    return;
                };
                if value.get("version").and_then(|v| v.as_u64()) != Some(1) || !value.is_object() {
                    return;
                }
                let mut entries = l.lock().unwrap();
                let previous = entries.get(&id).and_then(|(v, _)| v["frameID"].as_str());
                if previous != value["frameID"].as_str() {
                    frame_grids.lock().unwrap().remove(&id);
                }
                entries.retain(|_, (_, received): &mut (serde_json::Value, Instant)| {
                    received.elapsed().as_secs_f64() < SUPERVISION_STALE_SECONDS
                });
                if entries.len() < 4096 || entries.contains_key(&id) {
                    entries.insert(id, (value, Instant::now()));
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let operator_states: SharedTelemetry =
            Arc::new(Mutex::new(std::collections::BTreeMap::new()));
        let telemetry_cache = operator_states.clone();
        let heartbeat_maps = maps.clone();
        let subscribed_topics = topics.clone();
        let search_log = recorder.clone();
        let search_session = session_id.clone();
        let operator_feed: Handler = Arc::new(move |sample: zenoh::sample::Sample| {
            let key = sample.key_expr().as_str();
            let suffix = if key.ends_with("/map/occupancy/heartbeat") {
                "/map/occupancy/heartbeat"
            } else if key.ends_with("/pointcloud/heartbeat") {
                "/pointcloud/heartbeat"
            } else if key.ends_with("/hardware/status") {
                "/hardware/status"
            } else if key.ends_with("/pointcloud") {
                "/pointcloud"
            } else if key.ends_with("/autonomy/status") {
                "/autonomy/status"
            } else if key.ends_with("/pose/quality") {
                "/pose/quality"
            } else if key.ends_with("/navigation/outdoor") {
                "/navigation/outdoor"
            } else if key.ends_with("/search/report") {
                "/search/report"
            } else if key.ends_with("/search/status") {
                "/search/status"
            } else {
                return;
            };
            let Some(id) = subscribed_topics.rover_from_key(key, suffix) else {
                return;
            };
            let bytes = sample.payload().to_bytes();
            if suffix == "/map/occupancy/heartbeat" {
                if let Some(heartbeat) = content_heartbeat::occupancy_identity(&bytes, id)
                    && let Some(cached) = heartbeat_maps.lock().unwrap().get_mut(&id)
                {
                    content_heartbeat::refresh_grid(
                        cached,
                        &heartbeat,
                        start.elapsed().as_secs_f64(),
                    );
                }
                return;
            }
            if suffix == "/pointcloud/heartbeat" {
                if let Some(heartbeat) = content_heartbeat::cloud_identity(&bytes, id)
                    && let Some(cached) = telemetry_cache
                        .lock()
                        .unwrap()
                        .get_mut(&(id, "cloud".into()))
                {
                    content_heartbeat::refresh_cloud(cached, &heartbeat, Instant::now());
                }
                return;
            }
            if bytes.len() > 262144 {
                return;
            }
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                return;
            };
            if !value.is_object() {
                return;
            }
            if matches!(
                suffix,
                "/search/status" | "/search/report" | "/hardware/status"
            ) {
                let _ = search_log.record(serde_json::json!({
                    "kind": "state_observed",
                    "topic": suffix.trim_start_matches('/'),
                    "rover_id": id,
                    "time": start.elapsed().as_secs_f64(),
                    "utc": utc(),
                    "session_id": search_session,
                    "payload": value,
                }));
            }
            let kind = if suffix == "/search/report" {
                let Some(search) = value["search_id"].as_str() else {
                    return;
                };
                if !valid_token(search) {
                    return;
                }
                // Each search retains one report; batches replay on the canonical topic.
                let kind = format!("search_report:{search}");
                let mut entries = telemetry_cache.lock().unwrap();
                entries.retain(|_, (_, at)| at.elapsed().as_secs_f64() < 2.5);
                if entries.len() < 4096 || entries.contains_key(&(id, kind.clone())) {
                    entries.insert((id, kind), (value, Instant::now()));
                }
                return;
            } else if suffix == "/search/status" {
                "search"
            } else if suffix == "/hardware/status" {
                "hardware"
            } else if suffix == "/pointcloud" {
                "cloud"
            } else if suffix == "/map/occupancy" {
                "occupancy"
            } else if suffix == "/pose/quality" {
                "pose_quality"
            } else if suffix == "/navigation/outdoor" {
                "outdoor"
            } else {
                "authority"
            };
            let mut entries = telemetry_cache.lock().unwrap();
            entries.retain(|_, (_, at): &mut (serde_json::Value, Instant)| {
                at.elapsed().as_secs_f64() < SUPERVISION_STALE_SECONDS
            });
            if kind == "cloud"
                && entries
                    .get(&(id, kind.into()))
                    .is_some_and(|(old, _)| !content_heartbeat::accepts_cloud(old, &value))
            {
                return;
            }
            if entries.len() < 4096 || entries.contains_key(&(id, kind.into())) {
                entries.insert((id, kind.into()), (value, Instant::now()));
            }
        });
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
        let worker = {
            let packet_sequence = teleop_sequence.clone();
            let session = session.clone();
            let held = held.clone();
            let authority_cache = operator.clone();
            let hardware_cache = operator_states.clone();
            let fleet_cache = cache.clone();
            let stop = shutdown.clone();
            let prefix = topics.prefix.clone();
            let callback_session_id = session_id.clone();
            let log = recorder.clone();
            std::thread::spawn(move || {
                let mut last: Option<u64> = None;
                while !stop.load(std::sync::atomic::Ordering::Acquire) {
                    let now = start.elapsed().as_secs_f64();
                    let mut input = held.lock().unwrap();
                    let active = input.rover_id.filter(|id| {
                        let authority = authority_cache.lock().unwrap();
                        let permitted = authority.can_drive(*id, now)
                            && input
                                .authority_generation
                                .as_ref()
                                .is_none_or(|(run, revision)| {
                                    authority.matches_authority(*id, run, *revision)
                                });
                        drop(authority);
                        let hardware_ready = hardware_cache
                            .lock()
                            .unwrap()
                            .get(&(*id, "hardware".into()))
                            .is_some_and(|(value, at)| require_relay(value, at, false).is_ok());
                        permitted
                            && hardware_ready
                            && fleet_cache.lock().unwrap().snapshot(now).link == "healthy"
                    });
                    if active.is_none() {
                        input.rover_id = None;
                        input.keys.clear();
                        if let Some(guard) = input.guard.as_mut() {
                            guard.reset();
                        }
                    }
                    if input.deadman && input.lease_until > 0. && now >= input.lease_until {
                        input.keys.clear();
                    }
                    let dt = if input.last_step > 0. {
                        now - input.last_step
                    } else {
                        0.05
                    };
                    input.last_step = now;
                    let (unit_linear, unit_angular) = input.keys.sample();
                    // Defense in depth; Terra enforces the real limits.
                    let sample = if let Some(guard) = input.guard.as_mut() {
                        guard.step(unit_linear, unit_angular, dt)
                    } else {
                        (unit_linear, unit_angular)
                    };
                    if let Some(id) = active {
                        if sample != (0., 0.) || last.is_some() {
                            let sequence = packet_sequence
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                                + 1;
                            let payload = serde_json::json!({
                                "linear": sample.0,
                                "angular": sample.1,
                                "operator_session_id": callback_session_id,
                                "sequence": sequence,
                                "run_id": input.authority_generation.as_ref().map(|g|&g.0),
                                "authority_revision": input.authority_generation.as_ref().map(|g|g.1),
                            });
                            if let Some(s) = session.lock().unwrap().as_ref() {
                                let _ = s
                                    .put(format!("{prefix}/{id}/teleop"), payload.to_string())
                                    .wait();
                            }
                            let _ = log.record(serde_json::json!({
                                "kind": "teleop_sent",
                                "rover_id": id,
                                "time": now,
                                "utc": utc(),
                                "sequence": sequence,
                                "session_id": callback_session_id,
                                "payload": payload,
                            }));
                            last = if sample == (0., 0.) { None } else { Some(id) };
                        }
                    } else if let Some(id) = last.take()
                        && let Some(s) = session.lock().unwrap().as_ref()
                    {
                        let _ = s
                            .put(
                                format!("{prefix}/{id}/teleop"),
                                zero_payload(
                                    &callback_session_id,
                                    &packet_sequence,
                                    input.authority_generation.as_ref(),
                                ),
                            )
                            .wait();
                    }
                    drop(input);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            })
        };
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
        ) || payload.len() > 2048
        {
            return Err(invalid("invalid operator command"));
        }
        let value: serde_json::Value =
            serde_json::from_str(payload).map_err(|_| invalid("invalid command JSON"))?;
        if kind == "navigation/settings" {
            if value["token"]
                .as_str()
                .is_none_or(|v| v.is_empty() || v.len() > 64)
                || !value["run_id"].is_string()
                || value["authority_revision"].as_u64().is_none()
                || value["settings_revision"].as_u64().is_none()
                || !value["parameters"].is_object()
            {
                return Err(invalid("invalid navigation settings request"));
            }
        } else if matches!(kind, "navigation/source" | "navigation/resume") {
            if value["schema_version"] != 1
                || value["token"].as_str().is_none_or(|t| !valid_token(t))
                || !value["run_id"].is_string()
                || value["navigation_revision"].as_u64().is_none()
            {
                return Err(invalid("invalid navigation request"));
            }
        } else if kind.starts_with("search") {
            validate_search_command(kind, payload)?;
        } else if kind == "task/action" {
            argos_core::validate_task_action(payload)?;
        } else if kind == "teleop" {
            let linear = value["linear"]
                .as_f64()
                .ok_or_else(|| invalid("missing linear"))?;
            let angular = value["angular"]
                .as_f64()
                .ok_or_else(|| invalid("missing angular"))?;
            if !linear.is_finite()
                || !angular.is_finite()
                || linear.abs() > 0.5
                || angular.abs() > 1.
            {
                return Err(invalid("drive exceeds dashboard limits"));
            }
        } else if !matches!(
            value["level"].as_str(),
            Some(
                "teleop"
                    | "assisted_teleop"
                    | "waypoint"
                    | "waypoint_direct"
                    | "supervised"
                    | "target_search"
                    | "explore"
                    | "task"
            )
        ) {
            return Err(invalid("unsupported autonomy level"));
        } else if value["level"] == "explore"
            && value["budget_seconds"]
                .as_f64()
                .is_none_or(|seconds| !argos_core::valid_explore_budget(seconds))
        {
            return Err(invalid("explore needs a budget"));
        } else if value["level"] == "task"
            && value["task"]["instruction"]
                .as_str()
                .is_none_or(|text| argos_core::task_text(text).is_err())
        {
            return Err(invalid("L5 needs a task"));
        }
        // Neutral may still be sent during telemetry loss; moving commands require fresh fleet and pose.
        let neutral = kind == "search/report/ack"
            || (kind == "search/action"
                && matches!(value["action"].as_str(), Some("cancel" | "pause")))
            || (kind == "task/action" && value["action"] == "cancel")
            || kind == "teleop"
                && value["linear"].as_f64() == Some(0.)
                && value["angular"].as_f64() == Some(0.);
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
        let manual_without_pose = (kind == "autonomy" && value["level"] == "teleop")
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
            let states = self.operator_states.lock().unwrap();
            let (authority, received) = states
                .get(&(id, "authority".into()))
                .ok_or_else(|| invalid("vehicle authority unavailable"))?;
            if received.elapsed().as_secs_f64() >= freshness {
                return Err(invalid("vehicle authority is stale"));
            }
            if kind.starts_with("search") {
                if value["run_id"] != authority["run_id"]
                    || authority["requested_level"] != "target_search"
                {
                    return Err(invalid("search authority changed"));
                }
            } else if kind == "task/action" {
                if authority["requested_level"] != "task" {
                    return Err(invalid("Select L5 before replying to a task"));
                }
            } else if kind == "navigation/settings" {
                let (hardware, at) = states
                    .get(&(id, "hardware".into()))
                    .ok_or_else(|| invalid("hardware status unavailable"))?;
                require_relay(hardware, at, true)?;
                if value["run_id"] != authority["run_id"]
                    || value["authority_revision"] != authority["revision"]
                    || value["settings_revision"] != authority["navigation"]["revision"]
                    || authority["navigation"]["schema_version"] != 1
                    || !matches!(authority["safety"].as_str(), Some("clear" | "hold"))
                {
                    return Err(invalid("Settings require current vehicle authority"));
                }
            } else if kind == "navigation/resume" {
                let (raw, at) = states
                    .get(&(id, "outdoor".into()))
                    .ok_or_else(|| invalid("Outdoor status unavailable"))?;
                let mut report = argos_core::OutdoorReport::decode(raw)?;
                report.age_s = Some(at.elapsed().as_secs_f64());
                if let Some(reason) = argos_core::outdoor_resume_refusal(
                    &report,
                    value["run_id"].as_str().unwrap(),
                    value["navigation_revision"].as_u64().unwrap(),
                ) {
                    return Err(invalid(reason));
                }
                let (hardware, at) = states
                    .get(&(id, "hardware".into()))
                    .ok_or_else(|| invalid("Hardware unavailable"))?;
                require_relay(hardware, at, false)?;
                if authority["requested_level"] != "waypoint"
                    || authority["safety"] == "emergency_stop"
                {
                    return Err(invalid("Select L3 with ready hardware before resuming"));
                }
            } else if kind == "navigation/source" {
                let source = value["source"].as_str();
                if value["schema_version"] != 1
                    || !matches!(source, Some("local" | "outdoor_gps"))
                    || value["token"]
                        .as_str()
                        .is_none_or(|token| !valid_token(token))
                    || !value["run_id"].is_string()
                    || value["navigation_revision"].as_u64().is_none()
                {
                    return Err(invalid("invalid navigation source request"));
                }
                let (hardware, at) = states
                    .get(&(id, "hardware".into()))
                    .ok_or_else(|| invalid("hardware status unavailable"))?;
                require_relay(hardware, at, true)?;
                if value["run_id"] != authority["run_id"] {
                    return Err(invalid("navigation run changed"));
                }
                if self.snapshot().rovers.iter().any(|rover| {
                    rover.id == id
                        && rover
                            .goal
                            .as_ref()
                            .is_some_and(|goal| goal.state == GoalState::Active)
                }) {
                    return Err(invalid(
                        "Navigation source changes require no active mission",
                    ));
                }
                if source == Some("outdoor_gps") {
                    let (outdoor, outdoor_at) = states
                        .get(&(id, "outdoor".into()))
                        .ok_or_else(|| invalid("App update required"))?;
                    if outdoor_at.elapsed().as_secs_f64() >= 3.
                        || argos_core::OutdoorReport::decode(outdoor).is_err()
                        || argos_core::app_update_required(
                            argos_core::OutdoorReport::decode(outdoor).ok().as_ref(),
                        )
                        || outdoor["navigation_revision"] != value["navigation_revision"]
                        || outdoor["run_id"] != value["run_id"]
                    {
                        return Err(invalid("App update required"));
                    }
                }
            } else if kind == "teleop" {
                let (hardware, received) = states
                    .get(&(id, "hardware".into()))
                    .ok_or_else(|| invalid("hardware status unavailable"))?;
                require_relay(hardware, received, false)?;
                if value["run_id"] != authority["run_id"]
                    || value["authority_revision"] != authority["revision"]
                    || value["operator_session_id"]
                        .as_str()
                        .is_none_or(|s| s.is_empty() || s.len() > 64)
                    || value["sequence"].as_u64().is_none()
                    || !matches!(
                        authority["requested_level"].as_str(),
                        Some("teleop" | "assisted_teleop")
                    )
                    || authority["effective_level"] != authority["requested_level"]
                    || !matches!(authority["safety"].as_str(), Some("clear" | "active"))
                {
                    return Err(invalid("drive authority changed"));
                }
            } else if value["token"]
                .as_str()
                .is_none_or(|s| s.is_empty() || s.len() > 64)
                || !authority["supported_levels"]
                    .as_array()
                    .is_some_and(|levels| levels.contains(&value["level"]))
            {
                return Err(invalid("unsupported vehicle autonomy request"));
            }
        }
        let payload = self.bound_payload(id, kind, payload.as_bytes())?;
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
            let _ = self.recorder.record(serde_json::json!({
                "kind": if kind == "teleop" {"teleop_sent"} else {"action_sent"},
                "topic": kind,
                "rover_id": id,
                "token": value["token"],
                "time": self.start.elapsed().as_secs_f64(),
                "utc": utc(),
                "session_id": self.session_id,
                "payload": value,
            }));
        }
        result
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
            "time": now,
            "utc": utc(),
            "session_id": self.session_id,
            "payload": serde_json::from_slice::<serde_json::Value>(&payload).unwrap(),
        }));
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
        let mut payload = self.cache.lock().unwrap().prepare_goal(
            id,
            &goal,
            &token,
            self.start.elapsed().as_secs_f64(),
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
            "time": self.start.elapsed().as_secs_f64(),
            "utc": utc(),
            "session_id": self.session_id,
        }));
        self.publish(id, cancel_payload().to_vec())
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
