//! Zenoh publish/subscribe limited to FPV keys.
//!
//! The session is separate from [`argos_zenoh::Client`] so this path cannot call
//! the command, teleop, or hardware methods. Payloads are whatever
//! `fpv_signaling` put in a [`fpv_signaling::Effect::Publish`].

use fpv_signaling::{MAX_BYTES, Topic, topic_key};
use std::sync::Arc;
use tokio::sync::mpsc;
use zenoh::Wait;

pub struct Bus {
    session: zenoh::Session,
    prefix: String,
    subscribers: Vec<zenoh::pubsub::Subscriber<()>>,
    published: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Bus {
    pub fn connect(
        endpoint: &str,
        prefix: &str,
        credentials: Option<(&str, &str)>,
    ) -> Result<Self, String> {
        if endpoint.len() > 512 || !(endpoint.starts_with("tcp/") || endpoint.starts_with("tls/")) {
            return Err("FPV endpoint must be tcp/host:port or tls/host:port".into());
        }
        let mut config = zenoh::Config::default();
        config
            .insert_json5("mode", "\"client\"")
            .map_err(|error| error.to_string())?;
        config
            .insert_json5(
                "connect/endpoints",
                &serde_json::to_string(&vec![endpoint]).unwrap(),
            )
            .map_err(|error| error.to_string())?;
        config
            .insert_json5("scouting/multicast/enabled", "false")
            .map_err(|error| error.to_string())?;
        config
            .insert_json5("connect/timeout_ms", "3000")
            .map_err(|error| error.to_string())?;
        if let Some((user, password)) = credentials {
            for (key, value) in [
                ("transport/auth/usrpwd/user", user),
                ("transport/auth/usrpwd/password", password),
            ] {
                config
                    .insert_json5(key, &serde_json::to_string(value).unwrap())
                    .map_err(|error| error.to_string())?;
            }
        }
        let session = zenoh::open(config)
            .wait()
            .map_err(|error| format!("FPV link could not connect: {error}"))?;
        Ok(Self {
            session,
            prefix: prefix.trim_matches('/').to_string(),
            subscribers: Vec::new(),
            published: Arc::new(std::sync::Mutex::new(Vec::new())),
        })
    }

    pub fn subscribe(&mut self, phone_id: u64, topic: Topic) -> mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = mpsc::channel(64);
        let key = topic_key(&self.prefix, phone_id, topic);
        if let Ok(subscriber) = self
            .session
            .declare_subscriber(key)
            .callback(move |sample| {
                let bytes = sample.payload().to_bytes().to_vec();
                let _ = tx.try_send(bytes);
            })
            .wait()
        {
            self.subscribers.push(subscriber);
        }
        rx
    }

    /// Publish one signaling sample. `topic` is one of the six WebRTC suffixes.
    pub fn publish(&self, phone_id: u64, topic: Topic, body: &str) -> Result<(), String> {
        if body.len() > MAX_BYTES {
            return Err("FPV signaling payload exceeds 65536 bytes".into());
        }
        let key = topic_key(&self.prefix, phone_id, topic);
        self.put_raw(&key, body)
    }

    /// Telemetry for the local test sender so the dashboard can select a rover.
    /// These two keys are not motion commands.
    pub fn publish_announce(&self, phone_id: u64, sequence: u64) -> Result<(), String> {
        let fleet = format!("{}/fleet/state", self.prefix);
        let pose = format!("{}/{phone_id}/pose", self.prefix);
        let fleet_body = serde_json::json!({
            "count": 1,
            "max_count": 1,
            "ids": [phone_id],
        })
        .to_string();
        let pose_body = serde_json::json!({
            "rover_id": phone_id,
            "sequence": sequence,
            "x": 0.0,
            "y": 0.0,
            "yaw": 0.0,
        })
        .to_string();
        self.put_raw(&fleet, &fleet_body)?;
        self.put_raw(&pose, &pose_body)?;
        Ok(())
    }

    fn put_raw(&self, key: &str, body: &str) -> Result<(), String> {
        self.published.lock().unwrap().push(key.to_string());
        self.session
            .put(key, body)
            .wait()
            .map_err(|error| format!("FPV publish failed: {error}"))
    }
}
