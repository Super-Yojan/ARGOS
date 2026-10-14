//! Publishes N fake rovers on the ARGOS topic set.
//!
//! `--listen` opens the session the bridge connects to, so the spike can run
//! without Zorvane. Zorvane already publishes this same shape; use
//! `TERRA_ROVER_COUNT` for that path instead of stacking a second publisher
//! on `fleet/state`.

use argos_core::Topics;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{JoinHandle, sleep};
use std::time::Duration;
use zenoh::Wait;

use crate::{
    OCCUPANCY_RESOLUTION, OCCUPANCY_SIZE, occupancy_cells, occupancy_origin, rover_layout,
};

pub struct FleetPublisher {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for FleetPublisher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn serve(endpoint: &str, prefix: &str, count: u64) -> Result<FleetPublisher, String> {
    spawn_publisher(endpoint, prefix, count, true)
}

pub fn connect_publisher(
    endpoint: &str,
    prefix: &str,
    count: u64,
) -> Result<FleetPublisher, String> {
    spawn_publisher(endpoint, prefix, count, false)
}

fn spawn_publisher(
    endpoint: &str,
    prefix: &str,
    count: u64,
    listen: bool,
) -> Result<FleetPublisher, String> {
    validate(endpoint, prefix, count)?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let endpoint = endpoint.to_string();
    let prefix = prefix.to_string();
    let (ready_tx, ready_rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        if let Err(error) = run(&endpoint, &prefix, count, listen, &stop_thread, ready_tx) {
            eprintln!("ARGOS publisher stopped: {error}");
        }
    });
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(FleetPublisher {
            stop,
            thread: Some(thread),
        }),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(error)
        }
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            let _ = thread.join();
            Err("publisher did not become ready within 10s".into())
        }
    }
}

fn validate(endpoint: &str, prefix: &str, count: u64) -> Result<(), String> {
    if !(1..=32).contains(&count) {
        return Err("count must be 1..=32".into());
    }
    Topics::new(prefix).map_err(|error| error.to_string())?;
    if endpoint.len() > 512
        || !endpoint.starts_with("tcp/")
        || endpoint.parse::<zenoh::config::EndPoint>().is_err()
    {
        return Err("endpoint must be a valid tcp/host:port".into());
    }
    Ok(())
}

fn run(
    endpoint: &str,
    prefix: &str,
    count: u64,
    listen: bool,
    stop: &AtomicBool,
    ready: mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    let mut config = zenoh::Config::default();
    if listen {
        config
            .insert_json5(
                "listen/endpoints",
                &serde_json::json!([endpoint]).to_string(),
            )
            .map_err(|error| error.to_string())?;
    } else {
        config
            .insert_json5("mode", "\"client\"")
            .map_err(|error| error.to_string())?;
        config
            .insert_json5(
                "connect/endpoints",
                &serde_json::json!([endpoint]).to_string(),
            )
            .map_err(|error| error.to_string())?;
        config
            .insert_json5("connect/timeout_ms", "3000")
            .map_err(|error| error.to_string())?;
    }
    config
        .insert_json5("scouting/multicast/enabled", "false")
        .map_err(|error| error.to_string())?;
    let session = match zenoh::open(config).wait() {
        Ok(session) => session,
        Err(error) => {
            let message = format!("could not open zenoh: {error}");
            let _ = ready.send(Err(message.clone()));
            return Err(message);
        }
    };
    let mut sequence = 1u64;
    if let Err(error) = publish_once(&session, prefix, count, sequence) {
        let _ = ready.send(Err(error.clone()));
        return Err(error);
    }
    let mode = if listen { "listen" } else { "connect" };
    eprintln!(
        "ARGOS publisher ready endpoint={endpoint} prefix={prefix} count={count} mode={mode}"
    );
    let _ = ready.send(Ok(()));
    while !stop.load(Ordering::Relaxed) {
        sleep(Duration::from_millis(200));
        if stop.load(Ordering::Relaxed) {
            break;
        }
        sequence = sequence.saturating_add(1);
        publish_once(&session, prefix, count, sequence)?;
    }
    let _ = session.close().wait();
    Ok(())
}

fn publish_once(
    session: &zenoh::Session,
    prefix: &str,
    count: u64,
    sequence: u64,
) -> Result<(), String> {
    let topics = Topics::new(prefix).map_err(|error| error.to_string())?;
    let ids: Vec<u64> = (1..=count).collect();
    let fleet = serde_json::json!({
        "count": count,
        "max_count": 32,
        "ids": ids,
    });
    put(session, &topics.fleet(), fleet.to_string().into_bytes())?;
    for id in ids {
        let (x, y, yaw) = rover_layout(id);
        let pose = serde_json::json!({
            "rover_id": id,
            "sequence": sequence,
            "x": x,
            "y": y,
            "yaw": yaw,
        });
        put(
            session,
            &format!("{prefix}/{id}/pose"),
            pose.to_string().into_bytes(),
        )?;
        put(
            session,
            &format!("{prefix}/{id}/camera/depth"),
            depth_payload(id, sequence, x, y, yaw),
        )?;
        if let Some(status) = autonomy_status(id) {
            put(
                session,
                &format!("{prefix}/{id}/autonomy/status"),
                status.into_bytes(),
            )?;
        }
        put(
            session,
            &format!("{prefix}/{id}/map/occupancy"),
            occupancy_payload(id, sequence),
        )?;
    }
    Ok(())
}

fn occupancy_payload(id: u64, sequence: u64) -> Vec<u8> {
    let (origin_x, origin_y) = occupancy_origin(id);
    serde_json::json!({
        "schema_version": 1,
        "rover_id": id,
        "run_id": "spike",
        "sequence": sequence,
        "width": OCCUPANCY_SIZE,
        "height": OCCUPANCY_SIZE,
        "resolution": OCCUPANCY_RESOLUTION,
        "origin_x": origin_x,
        "origin_y": origin_y,
        "occupancy": occupancy_cells(id),
    })
    .to_string()
    .into_bytes()
}

fn autonomy_status(id: u64) -> Option<String> {
    let body = match id {
        2 => serde_json::json!({
            "requested_level": "teleop",
            "effective_level": "teleop",
            "active_source": "none",
            "safety": "hold",
            "reason": "obstacle_blocked",
            "revision": 1,
            "run_id": "spike",
            "supported_levels": ["teleop"],
            "paused": false
        }),
        3 => serde_json::json!({
            "requested_level": "teleop",
            "effective_level": "teleop",
            "active_source": "none",
            "safety": "emergency_stop",
            "reason": "estop",
            "revision": 1,
            "run_id": "spike",
            "supported_levels": ["teleop"],
            "paused": false
        }),
        _ => return None,
    };
    Some(body.to_string())
}

fn depth_payload(id: u64, sequence: u64, x: f64, y: f64, yaw: f64) -> Vec<u8> {
    let header = serde_json::json!({
        "version": 1,
        "encoding": "32FC1_LE",
        "rover_id": id,
        "sequence": sequence,
        "body": {"x": x, "y": y, "yaw": yaw}
    });
    let mut bytes = header.to_string().into_bytes();
    bytes.push(b'\n');
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes
}

fn put(session: &zenoh::Session, key: &str, payload: Vec<u8>) -> Result<(), String> {
    session
        .put(key, payload)
        .wait()
        .map_err(|error| format!("put {key} failed: {error}"))
}
