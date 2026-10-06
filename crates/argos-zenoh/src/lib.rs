use argos_core::*;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use zenoh::Wait;
/// One native session; subscriptions retain only latest parsed telemetry.
pub struct Client {
    session: Mutex<Option<zenoh::Session>>,
    subscribers: Mutex<Vec<zenoh::pubsub::Subscriber<()>>>,
    cache: Arc<Mutex<FleetCache>>,
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
        Ok(Self {
            session: Mutex::new(Some(session)),
            subscribers: Mutex::new(vec![fleet, status, depth]),
            cache,
            actions: Mutex::new(()),
            topics,
            start,
        })
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
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.disconnect();
    }
}
