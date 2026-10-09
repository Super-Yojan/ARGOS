use argos_core::*;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use zenoh::Wait;
/// One native session; subscriptions retain only latest parsed telemetry.
pub struct Client {
    session: Mutex<Option<zenoh::Session>>,
    subscribers: Mutex<Vec<zenoh::pubsub::Subscriber<()>>>,
    cache: Arc<Mutex<FleetCache>>,
    localizations: Arc<Mutex<std::collections::BTreeMap<u64, (serde_json::Value, Instant)>>>,
    operator_states:
        Arc<Mutex<std::collections::BTreeMap<(u64, String), (serde_json::Value, Instant)>>>,
    actions: Mutex<()>,
    topics: Topics,
    start: Instant,
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
        let c = cache.clone();
        let t = topics.clone();
        let pose = session
            .declare_subscriber(format!("{}/*/pose", topics.prefix))
            .callback(move |sample| {
                let Some(id) = t.rover_from_key(sample.key_expr().as_str(), "/pose") else {
                    return;
                };
                let payload = sample.payload().to_bytes();
                let mut cache = c.lock().unwrap();
                match decode_phone_pose(&payload) {
                    Ok(p) => cache.pose(id, p, start.elapsed().as_secs_f64()),
                    Err(e) => cache.notice(e.to_string()),
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
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
                    received.elapsed().as_secs_f64() < 2.5
                });
                if entries.len() < 4096 || entries.contains_key(&id) {
                    entries.insert(id, (value, Instant::now()));
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        let operator_states: Arc<Mutex<std::collections::BTreeMap<(u64,String),(serde_json::Value,Instant)>>> = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
        let o = operator_states.clone();
        let t = topics.clone();
        let operator = session
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
                } else if key.ends_with("/map/occupancy") {
                    "/map/occupancy"
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
                let kind = if suffix == "/search/report" {
                    let Some(search)=value["search_id"].as_str() else {return;};
                    if !valid_token(search){return;}
                    // Each search retains one report; batches replay on the canonical topic.
                    let kind=format!("search_report:{search}");
                    let mut entries=o.lock().unwrap();entries.retain(|_,(_,at)|at.elapsed().as_secs_f64()<2.5);
                    if entries.len()<4096||entries.contains_key(&(id,kind.clone())){entries.insert((id,kind),(value,Instant::now()));}
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
                    at.elapsed().as_secs_f64() < 2.5
                });
                if entries.len() < 4096 || entries.contains_key(&(id, kind.into())) {
                    entries.insert((id, kind.into()), (value, Instant::now()));
                }
            })
            .wait()
            .map_err(|e| Error(e.to_string()))?;
        Ok(Self {
            session: Mutex::new(Some(session)),
            subscribers: Mutex::new(vec![fleet, status, depth, pose, localization, operator]),
            cache,
            localizations,
            operator_states,
            actions: Mutex::new(()),
            topics,
            start,
        })
    }
    pub fn operator_snapshot(&self) -> String {
        let mut result = serde_json::json!({});
        for ((id, kind), (value, received)) in self.operator_states.lock().unwrap().iter() {
            let age = received.elapsed().as_secs_f64();
            if age >= 2.5 {
                continue;
            }
            let mut value = value.clone();
            value["age"] = serde_json::json!(age);
            if kind.starts_with("search_report:") {
                if !result[id.to_string()]["reports"].is_array(){result[id.to_string()]["reports"]=serde_json::json!([]);}
                result[id.to_string()]["reports"].as_array_mut().unwrap().push(value);
            }else{result[id.to_string()][kind] = value;}
        }
        result.to_string()
    }
    pub fn operator_command(&self, id: u64, kind: &str, payload: &str) -> Result<()> {
        let _action = self.actions.lock().unwrap();
        if !matches!(kind, "autonomy" | "teleop" | "hardware" | "search" | "search/action" | "search/report/ack") || payload.len() > 2048 {
            return Err(invalid("invalid operator command"));
        }
        let value: serde_json::Value =
            serde_json::from_str(payload).map_err(|_| invalid("invalid command JSON"))?;
        if kind.starts_with("search") {validate_search_command(kind,payload)?;} else if kind == "teleop" {
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
                || value["token"].as_str().is_none_or(|s| s.is_empty() || s.len() > 64) {
                return Err(invalid("invalid hardware request"));
            }
        } else if !matches!(
            value["level"].as_str(),
            Some("teleop" | "assisted_teleop" | "waypoint" | "supervised" | "target_search")
        ) {
            return Err(invalid("unsupported autonomy level"));
        }
        // Neutral may still be sent during telemetry loss; moving commands require fresh fleet and pose.
        let neutral = kind=="search/report/ack" || (kind=="search/action" && matches!(value["action"].as_str(),Some("cancel"|"pause"))) || (kind == "hardware" && value["action"] == "disarm") || kind == "teleop"
            && value["linear"].as_f64() == Some(0.)
            && value["angular"].as_f64() == Some(0.);
        if !neutral {
            let snapshot = self.snapshot();
            if snapshot.link != "healthy"
                || !snapshot.rovers.iter().any(|r| {
                    r.id == id && r.membership == "online" && r.pose_age.is_some_and(|a| a < 0.5)
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
            if received.elapsed().as_secs_f64() >= 0.5 {
                return Err(invalid("vehicle authority is stale"));
            }
            if kind.starts_with("search") {if value["run_id"]!=authority["run_id"] || authority["requested_level"]!="target_search" {return Err(invalid("search authority changed"));}} else if kind == "hardware" {
                if value["run_id"] != authority["run_id"] || value["authority_revision"] != authority["revision"]
                    || authority["safety"] != "clear" { return Err(invalid("arm authority changed")); }
            } else if kind == "teleop" {
                let (hardware, received) = states.get(&(id, "hardware".into())).ok_or_else(|| invalid("hardware status unavailable"))?;
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
        self.session
            .lock()
            .unwrap()
            .as_ref()
            .ok_or_else(|| invalid("disconnected"))?
            .put(format!("{}/{id}/{kind}", self.topics.prefix), payload)
            .wait()
            .map_err(|e| Error(e.to_string()))
    }
    pub fn localization_snapshot(&self) -> String {
        let values: serde_json::Map<String, serde_json::Value> = self
            .localizations
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(id, (value, received))| {
                let age = received.elapsed().as_secs_f64();
                if age >= 2.5 {
                    return None;
                }
                let mut value = value.clone();
                value["age"] = serde_json::json!(age);
                Some((id.to_string(), value))
            })
            .collect();
        serde_json::Value::Object(values).to_string()
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
        self.publish(id, payload)?;
        Ok(token)
    }
    pub fn cancel_goal(&self, id: u64) -> Result<()> {
        let _action = self.actions.lock().unwrap();
        self.cache
            .lock()
            .unwrap()
            .prepare_cancel(id, self.start.elapsed().as_secs_f64())?;
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
        self.subscribers.lock().unwrap().clear();
        if let Some(session) = self.session.lock().unwrap().take() {
            let _ = session.close().wait();
        }
        self.cache.lock().unwrap().disconnected();
        self.localizations.lock().unwrap().clear();
        self.operator_states.lock().unwrap().clear();
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.disconnect();
    }
}
