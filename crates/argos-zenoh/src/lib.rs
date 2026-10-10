pub mod router;
mod session_log;
use argos_core::*;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use zenoh::Wait;
type SharedTelemetry =
    Arc<Mutex<std::collections::BTreeMap<(u64, String), (serde_json::Value, Instant)>>>;
type InputState = (Option<u64>, HeldInput, Option<(String, u64)>);
/// One native session; subscriptions retain only latest parsed telemetry.
pub struct Client {
    session: Arc<Mutex<Option<zenoh::Session>>>,
    subscribers: Mutex<Vec<zenoh::pubsub::Subscriber<()>>>,
    cache: Arc<Mutex<FleetCache>>,
    maps: Arc<Mutex<std::collections::BTreeMap<u64, (OccupancyGrid, f64)>>>,
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
}
impl Client {
    pub fn connect(endpoint: &str, prefix: &str) -> Result<Self> {
        let topics = Topics::new(prefix)?;
        if endpoint.len() > 512
            || !endpoint.starts_with("tcp/")
            || endpoint.parse::<zenoh::config::EndPoint>().is_err()
        {
            return Err(invalid("endpoint must be a valid tcp/host:port"));
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
        let session = zenoh::open(config)
            .wait()
            .map_err(|e| Error(format!("could not connect: {e}")))?;
        let cache = Arc::new(Mutex::new(FleetCache::default()));
        let start = Instant::now();
        let c = cache.clone();
        let fleet = session
            .declare_subscriber(topics.fleet())
            .callback(move |sample| {
                let payload = sample.payload().to_bytes();
                let mut cache = c.lock().unwrap();
                match decode_fleet(&payload) {
                    Ok(f) => cache.fleet(f, start.elapsed().as_secs_f64()),
                    Err(e) => cache.notice(e.to_string()),
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let c = cache.clone();
        let t = topics.clone();
        let status = session
            .declare_subscriber(topics.statuses())
            .callback(move |sample| {
                let Some(id) = t.rover_from_key(sample.key_expr().as_str(), "/goal/status") else {
                    return;
                };
                let payload = sample.payload().to_bytes();
                let mut cache = c.lock().unwrap();
                match decode_status(&payload) {
                    Ok(s) => cache.status(id, s, start.elapsed().as_secs_f64()),
                    Err(e) => cache.notice(e.to_string()),
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let c = cache.clone();
        let t = topics.clone();
        let depth = session
            .declare_subscriber(topics.depth())
            .callback(move |sample| {
                let Some(id) = t.rover_from_key(sample.key_expr().as_str(), "/camera/depth") else {
                    return;
                };
                let payload = sample.payload().to_bytes();
                let mut cache = c.lock().unwrap();
                match decode_pose(&payload) {
                    Ok(Some(p)) => cache.pose(id, p, start.elapsed().as_secs_f64()),
                    Ok(None) => {}
                    Err(e) => cache.notice(e.to_string()),
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let direct_cache = cache.clone();
        let direct_topics = topics.clone();
        let direct_pose = session
            .declare_subscriber(format!("{}/*/pose", topics.prefix))
            .callback(move |sample| {
                if let Some(id) = direct_topics.rover_from_key(sample.key_expr().as_str(), "/pose")
                    && let Ok(pose) = decode_body_pose(&sample.payload().to_bytes())
                {
                    direct_cache
                        .lock()
                        .unwrap()
                        .pose(id, pose, start.elapsed().as_secs_f64());
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let maps = Arc::new(Mutex::new(std::collections::BTreeMap::<
            u64,
            (OccupancyGrid, f64),
        >::new()));
        let grids = maps.clone();
        let map_topics = topics.clone();
        let occupancy = session
            .declare_subscriber(format!("{}/*/map/occupancy", topics.prefix))
            .callback(move |sample| {
                let Some(id) =
                    map_topics.rover_from_key(sample.key_expr().as_str(), "/map/occupancy")
                else {
                    return;
                };
                let Ok(grid) = decode_occupancy(&sample.payload().to_bytes()) else {
                    return;
                };
                if grid.rover_id != id {
                    return;
                }
                let mut maps = grids.lock().unwrap();
                if maps.get(&id).is_some_and(|(old, _)| {
                    old.run_id == grid.run_id && old.sequence >= grid.sequence
                }) {
                    return;
                }
                // Keep memory bounded even if an endpoint publishes unlisted IDs.
                if !maps.contains_key(&id) && maps.len() >= 256 {
                    return;
                }
                maps.insert(id, (grid, start.elapsed().as_secs_f64()));
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let operator = Arc::new(Mutex::new(OperatorCache::default()));
        let held = Arc::new(Mutex::new(InputState::default()));
        let restart_input = held.clone();
        let session_id = uuid::Uuid::new_v4().to_string();
        let log_path = std::env::temp_dir().join(format!("argos-session-{session_id}.jsonl"));
        let recorder=Arc::new(session_log::RunRecorder::create(&log_path,serde_json::json!({"session_id":session_id,"utc":utc(),"platform":"argos","clock":"session_relative_monotonic"}),8192).unwrap_or_else(|_|session_log::RunRecorder::unavailable()));
        let op = operator.clone();
        let log = recorder.clone();
        let t = topics.clone();
        let c = cache.clone();
        let sid = session_id.clone();
        let authority=session.declare_subscriber(format!("{}/*/**",topics.prefix))
            .callback(move |sample|{
                for kind in ["autonomy/status","goal/proposal","mission/status","experiment/status"] {
                    let Some(id)=t.rover_from_key(sample.key_expr().as_str(),&format!("/{kind}"))else{continue};
                    if !c.lock().unwrap().snapshot(start.elapsed().as_secs_f64()).rovers.iter().any(|r|r.id==id&&r.membership!="absent"){return;}
                    let bytes=sample.payload().to_bytes();
                    let now=start.elapsed().as_secs_f64();
                    let (ok,changed)={
                        let mut state=op.lock().unwrap();
                        let previous=state.run_id(id);
                        let ok=state.apply(id,kind,&bytes,now).is_ok();
                        (ok,previous!=state.run_id(id))
                    };
                    if changed {
                        let mut input=restart_input.lock().unwrap();
                        if input.0==Some(id){input.0=None;input.1.clear();input.2=None;}
                    }
                    if ok {
                        let _=log.record(serde_json::json!({"kind":"state_observed","topic":kind,"rover_id":id,"time":now,"utc":utc(),"session_id":sid,"payload":serde_json::from_slice::<serde_json::Value>(&bytes).ok()}));
                    }
                    return;
                }
            }).wait().map_err(|e|Error(e.to_string()))?;
        let localizations = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
        let l = localizations.clone();
        let t = topics.clone();
        let localization = session
            .declare_subscriber(format!("{}/*/localization", topics.prefix))
            .callback(move |sample| {
                let Some(id) = t.rover_from_key(sample.key_expr().as_str(), "/localization") else {
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
        let o = operator_states.clone();
        let t = topics.clone();
        let search_log = recorder.clone();
        let search_session = session_id.clone();
        let operator_feed = session
            .declare_subscriber(format!("{}/*/**", topics.prefix))
            .callback(move |sample| {
                let key = sample.key_expr().as_str();
                let suffix = if key.ends_with("/hardware/status") {
                    "/hardware/status"
                } else if key.ends_with("/pointcloud") {
                    "/pointcloud"
                } else if key.ends_with("/autonomy/status") {
                    "/autonomy/status"
                } else if key.ends_with("/search/report") {
                    "/search/report"
                } else if key.ends_with("/search/status") {
                    "/search/status"

                } else {
                    return;
                };
                let Some(id) = t.rover_from_key(key, suffix) else {
                    return;
                };
                let bytes = sample.payload().to_bytes();
                if bytes.len() > 262144 {
                    return;
                }
                let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    return;
                };
                if !value.is_object() {
                    return;
                }
                if matches!(suffix, "/search/status" | "/search/report" | "/hardware/status") {
                    let _ = search_log.record(serde_json::json!({"kind":"state_observed","topic":suffix.trim_start_matches('/'),"rover_id":id,"time":start.elapsed().as_secs_f64(),"utc":utc(),"session_id":search_session,"payload":value}));
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
                    let mut entries = o.lock().unwrap();
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
                } else {
                    "authority"
                };
                let mut entries = o.lock().unwrap();
                entries.retain(|_, (_, at): &mut (serde_json::Value, Instant)| {
                    at.elapsed().as_secs_f64() < SUPERVISION_STALE_SECONDS
                });
                if entries.len() < 4096 || entries.contains_key(&(id, kind.into())) {
                    entries.insert((id, kind.into()), (value, Instant::now()));
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let session = Arc::new(Mutex::new(Some(session)));
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let teleop_sequence = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let worker = {
            let packet_sequence = teleop_sequence.clone();
            let session = session.clone();
            let held = held.clone();
            let op = operator.clone();
            let c = cache.clone();
            let stop = shutdown.clone();
            let prefix = topics.prefix.clone();
            let sid = session_id.clone();
            let log = recorder.clone();
            std::thread::spawn(move || {
                let mut last: Option<u64> = None;
                while !stop.load(std::sync::atomic::Ordering::Acquire) {
                    let now = start.elapsed().as_secs_f64();
                    let mut input = held.lock().unwrap();
                    let active = input.0.filter(|id| {
                        let authority = op.lock().unwrap();
                        let permitted = authority.can_drive(*id, now)
                            && input.2.as_ref().is_none_or(|(run, revision)| {
                                authority.matches_authority(*id, run, *revision)
                            });
                        drop(authority);
                        permitted && c.lock().unwrap().snapshot(now).link == "healthy"
                    });
                    if active.is_none() {
                        input.0 = None;
                        input.1.clear();
                    }
                    let sample = input.1.sample();
                    if let Some(id) = active {
                        if sample != (0., 0.) || last.is_some() {
                            let sequence = packet_sequence
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                                + 1;
                            let payload = serde_json::json!({"linear":sample.0,"angular":sample.1,"operator_session_id":sid,"sequence":sequence,"run_id":input.2.as_ref().map(|g|&g.0),"authority_revision":input.2.as_ref().map(|g|g.1)});
                            if let Some(s) = session.lock().unwrap().as_ref() {
                                let _ = s
                                    .put(format!("{prefix}/{id}/teleop"), payload.to_string())
                                    .wait();
                            }
                            let _=log.record(serde_json::json!({"kind":"teleop_sent","rover_id":id,"time":now,"utc":utc(),"sequence":sequence,"session_id":sid,"payload":payload}));
                            last = if sample == (0., 0.) { None } else { Some(id) };
                        }
                    } else if let Some(id) = last.take()
                        && let Some(s) = session.lock().unwrap().as_ref()
                    {
                        let _ = s
                            .put(
                                format!("{prefix}/{id}/teleop"),
                                zero_payload(&sid, &packet_sequence, input.2.as_ref()),
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
            subscribers: Mutex::new(vec![
                fleet,
                status,
                depth,
                authority,
                direct_pose,
                occupancy,
                localization,
                operator_feed,
            ]),
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
        })
    }
    pub fn operator_snapshot(&self) -> String {
        let mut result: serde_json::Value = serde_json::from_str(&self.legacy_operator_snapshot())
            .unwrap_or_else(|_| serde_json::json!({}));
        for ((id, kind), (value, received)) in self.operator_states.lock().unwrap().iter() {
            let age = received.elapsed().as_secs_f64();
            let max_age = if kind == "authority" || kind == "hardware" { SUPERVISION_STALE_SECONDS } else { 2.5 };
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
        let maps: serde_json::Value =
            serde_json::from_str(&self.occupancy_snapshot()).unwrap_or_default();
        if let Some(maps) = maps.as_object() {
            for (id, entry) in maps {
                let mut grid = entry["grid"].clone();
                grid["age"] = entry["age"].clone();
                result[id]["occupancy"] = grid;
            }
        }
        result.to_string()
    }
    pub fn operator_command(&self, id: u64, kind: &str, payload: &str) -> Result<()> {
        let _action = self.actions.lock().unwrap();
        if !matches!(
            kind,
            "autonomy" | "teleop" | "hardware" | "search" | "search/action" | "search/report/ack"
        ) || payload.len() > 2048
        {
            return Err(invalid("invalid operator command"));
        }
        let value: serde_json::Value =
            serde_json::from_str(payload).map_err(|_| invalid("invalid command JSON"))?;
        if kind.starts_with("search") {
            validate_search_command(kind, payload)?;
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
        } else if kind == "hardware" {
            if !matches!(value["action"].as_str(), Some("arm" | "disarm"))
                || value["token"]
                    .as_str()
                    .is_none_or(|s| s.is_empty() || s.len() > 64)
            {
                return Err(invalid("invalid hardware request"));
            }
        } else if !matches!(
            value["level"].as_str(),
            Some("teleop" | "assisted_teleop" | "waypoint" | "waypoint_direct" | "supervised" | "target_search")
        ) {
            return Err(invalid("unsupported autonomy level"));
        }
        // Neutral may still be sent during telemetry loss; moving commands require fresh fleet and pose.
        let neutral = kind == "search/report/ack"
            || (kind == "search/action"
                && matches!(value["action"].as_str(), Some("cancel" | "pause")))
            || (kind == "hardware" && value["action"] == "disarm")
            || kind == "teleop"
                && value["linear"].as_f64() == Some(0.)
                && value["angular"].as_f64() == Some(0.);
        let freshness = if kind == "autonomy" && matches!(value["level"].as_str(), Some("waypoint" | "waypoint_direct")) { SUPERVISION_STALE_SECONDS } else { 0.5 };
        if !neutral {
            let snapshot = self.snapshot();
            if snapshot.link != "healthy"
                || !snapshot.rovers.iter().any(|r| {
                    r.id == id && r.membership == "online" && r.pose_age.is_some_and(|a| a < freshness)
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
            } else if kind == "hardware" {
                if value["run_id"] != authority["run_id"]
                    || value["authority_revision"] != authority["revision"]
                    || authority["safety"] != "clear"
                {
                    return Err(invalid("arm authority changed"));
                }
            } else if kind == "teleop" {
                let (hardware, received) = states
                    .get(&(id, "hardware".into()))
                    .ok_or_else(|| invalid("hardware status unavailable"))?;
                if received.elapsed().as_secs_f64() >= 0.5 || hardware["armed"] != true {
                    return Err(invalid("motors are not confirmed armed"));
                }
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
            let _ = self.recorder.record(serde_json::json!({"kind":if kind == "teleop" {"teleop_sent"} else {"action_sent"},"topic":kind,"rover_id":id,"token":value["token"],"time":self.start.elapsed().as_secs_f64(),"utc":utc(),"session_id":self.session_id,"payload":value}));
        }
        result
    }
    pub fn localization_snapshot(&self) -> String {
        let values: serde_json::Map<String, serde_json::Value> = self
            .localizations
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(id, (value, received))| {
                let age = received.elapsed().as_secs_f64();
                if age >= SUPERVISION_STALE_SECONDS {
                    return None;
                }
                let mut value = value.clone();
                value["age"] = serde_json::json!(age);
                Some((id.to_string(), value))
            })
            .collect();
        serde_json::Value::Object(values).to_string()
    }
    pub fn occupancy_snapshot(&self) -> String {
        let now = self.start.elapsed().as_secs_f64();
        let fleet = self.cache.lock().unwrap().snapshot(now);
        let maps = self.maps.lock().unwrap();
        let mut result = serde_json::Map::new();
        for rover in fleet.rovers.iter().filter(|r| r.membership != "absent") {
            if let Some((grid, time)) = maps.get(&rover.id) {
                result.insert(
                    rover.id.to_string(),
                    serde_json::json!({"grid":grid,"age":(now-time).max(0.)}),
                );
            }
        }
        serde_json::Value::Object(result).to_string()
    }
    fn legacy_operator_snapshot(&self) -> String {
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
        state.to_string()
    }
    pub fn action(&self, id: u64, kind: &str, value: serde_json::Value) -> Result<String> {
        let token = uuid::Uuid::new_v4().to_string();
        let payload = operator_payload(kind, value, &token)?;
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
        let _=self.recorder.record(serde_json::json!({"kind":"action_sent","rover_id":id,"token":token,"topic":kind,"time":now,"utc":utc(),"session_id":self.session_id,"payload":serde_json::from_slice::<serde_json::Value>(&payload).unwrap()}));
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
        let _=self.recorder.record(serde_json::json!({"kind":"fleet_action","parent_action_id":parent,"targets":targets,"time":self.start.elapsed().as_secs_f64(),"utc":utc()}));
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
        let accepted_generation = {
            let authority = self.operator.lock().unwrap();
            if generation
                .is_some_and(|(run, revision)| !authority.matches_authority(id, run, revision))
            {
                return Ok(());
            }
            if down && !authority.can_drive(id, now) {
                return Err(invalid("fresh confirmed teleop authority required"));
            }
            authority.generation(id)
        };
        if held.0 != Some(id) {
            if let Some(previous) = held.0
                && let Some(s) = self.session.lock().unwrap().as_ref()
            {
                let _ = s
                    .put(
                        format!("{}/{previous}/teleop", self.topics.prefix),
                        zero_payload(&self.session_id, &self.teleop_sequence, held.2.as_ref()),
                    )
                    .wait();
            }
            held.1.clear();
        }
        held.0 = Some(id);
        held.2 = accepted_generation;
        if down {
            held.1.press(key);
        } else {
            held.1.release(key);
        }
        Ok(())
    }
    pub fn clear_input(&self) {
        let mut held = self.held.lock().unwrap();
        let id = held.0.take();
        held.1.clear();
        if let Some(id) = id
            && let Some(s) = self.session.lock().unwrap().as_ref()
        {
            let _ = s
                .put(
                    format!("{}/{id}/teleop", self.topics.prefix),
                    zero_payload(&self.session_id, &self.teleop_sequence, held.2.as_ref()),
                )
                .wait();
        }
    }
    pub fn export_session(&self, destination: &str) -> Result<()> {
        let log = &self.recorder;
        log.flush().map_err(|e| Error(e.to_string()))?;
        let _=log.record(serde_json::json!({"kind":"session_snapshot","recording_failed":log.failed(),"time":self.start.elapsed().as_secs_f64(),"utc":utc()}));
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
        let _action = self.actions.lock().unwrap();
        let token = uuid::Uuid::new_v4().to_string();
        let payload = self.cache.lock().unwrap().prepare_goal(
            id,
            &goal,
            &token,
            self.start.elapsed().as_secs_f64(),
        )?;
        self.publish(id, payload.clone())?;
        let _=self.recorder.record(serde_json::json!({"kind":"goal_sent","rover_id":id,"token":token,"time":self.start.elapsed().as_secs_f64(),"utc":utc(),"session_id":self.session_id,"payload":serde_json::from_slice::<serde_json::Value>(&payload).ok()}));
        Ok(token)
    }
    pub fn cancel_goal(&self, id: u64) -> Result<()> {
        let _action = self.actions.lock().unwrap();
        self.cache
            .lock()
            .unwrap()
            .prepare_cancel(id, self.start.elapsed().as_secs_f64())?;
        let _=self.recorder.record(serde_json::json!({"kind":"goal_cancel_sent","rover_id":id,"time":self.start.elapsed().as_secs_f64(),"utc":utc(),"session_id":self.session_id}));
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
        let _action = self.actions.lock().unwrap();
        self.clear_input();
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(w) = self.worker.lock().unwrap().take() {
            let _ = w.join();
        }
        self.subscribers.lock().unwrap().clear();
        let _ = self.recorder.close();
        if let Some(session) = self.session.lock().unwrap().take() {
            let _ = session.close().wait();
        }
        self.cache.lock().unwrap().disconnected();
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
    serde_json::json!({"linear":0,"angular":0,"operator_session_id":session,"sequence":sequence,"run_id":generation.map(|g|&g.0),"authority_revision":generation.map(|g|g.1)})
        .to_string()
}

#[cfg(test)] mod forest_tests {
    use super::*;
    use std::time::Duration;
    #[test] fn waypoint_mode_accepts_delayed_reports_but_manual_mode_does_not() {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port(); drop(probe);
        let router = router::Router::default(); router.start(port, "").unwrap();
        let mut client = Client::connect(&format!("tcp/127.0.0.1:{port}"), "forest-test").unwrap();
        client.start = Instant::now() - Duration::from_secs(60);
        {
            let mut cache = client.cache.lock().unwrap();
            cache.fleet(decode_fleet(br#"{"count":1,"max_count":32,"ids":[8]}"#).unwrap(), 0.);
            cache.pose(8, Pose { rover_id: 8, sequence: 1, x: 0., y: 0., yaw: 0. }, 0.);
        }
        client.operator_states.lock().unwrap().insert((8,"authority".into()),
            (serde_json::json!({"run_id":"run","revision":1,"requested_level":"teleop","supported_levels":["teleop","waypoint","waypoint_direct"],"safety":"clear"}), Instant::now()-Duration::from_secs(60)));
        assert!(client.operator_command(8,"autonomy",r#"{"level":"waypoint","token":"delayed"}"#).is_ok());
        assert!(client.operator_command(8,"autonomy",r#"{"level":"waypoint_direct","token":"delayed-direct"}"#).is_ok());
        assert!(client.operator_command(8,"autonomy",r#"{"level":"teleop","token":"manual"}"#).is_err());
        client.disconnect(); router.stop().unwrap();
    }
}
