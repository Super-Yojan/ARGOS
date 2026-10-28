//! Loopback Zenoh router for dashboard screenshots.
//!
//! Listens on `tcp/127.0.0.1:7447` with prefix `terra/rover` (the `--sim`
//! endpoint) and publishes three rovers. Operator commands on `autonomy`,
//! `safety`, and `goal` are applied on the next tick so Take over, Drive,
//! and Stop can be captured from the live window.
use serde_json::{Value, json};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};
use zenoh::Wait;

struct Goal {
    x: f64,
    y: f64,
    state: &'static str,
    distance: f64,
}

struct Rover {
    id: u64,
    x: f64,
    y: f64,
    yaw: f64,
    requested: &'static str,
    effective: &'static str,
    safety: &'static str,
    revision: u64,
    goal: Option<Goal>,
    route: Vec<(f64, f64, f64)>,
}

fn main() {
    let endpoint =
        std::env::var("ARGOS_FIXTURE_ENDPOINT").unwrap_or_else(|_| "tcp/127.0.0.1:7447".into());
    let mut config = zenoh::Config::default();
    config.insert_json5("mode", "\"router\"").unwrap();
    config
        .insert_json5("listen/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    config
        .insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    let session = zenoh::open(config)
        .wait()
        .unwrap_or_else(|error| panic!("fixture router failed: {error}"));
    let (tx, rx) = mpsc::channel();
    let mut kept = Vec::new();
    for kind in ["autonomy", "safety", "goal"] {
        let tx = tx.clone();
        let subscriber = session
            .declare_subscriber(format!("terra/rover/*/{kind}"))
            .callback(move |sample| {
                let key = sample.key_expr().as_str().to_string();
                let bytes = sample.payload().to_bytes().to_vec();
                let _ = tx.send((key, bytes));
            })
            .wait()
            .unwrap_or_else(|error| panic!("subscribe {kind}: {error}"));
        kept.push(subscriber);
    }
    // Subscribers have to stay alive for the life of the router.
    let _subscribers = kept;
    let mut rovers = seed();
    println!("screenshot fixture listening on {endpoint} prefix terra/rover");
    let started = Instant::now();
    let mut tick: u64 = 0;
    loop {
        apply_commands(&rx, &mut rovers);
        publish(&session, &rovers, tick);
        tick = tick.wrapping_add(1);
        if tick % 25 == 0 {
            println!(
                "fixture tick {tick} up {:.0}s",
                started.elapsed().as_secs_f64()
            );
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn seed() -> Vec<Rover> {
    vec![
        Rover {
            id: 1,
            x: 0.,
            y: 0.,
            yaw: 0.6,
            requested: "waypoint",
            effective: "waypoint",
            safety: "clear",
            revision: 3,
            goal: Some(goal(0., 0., 18., 10.)),
            route: vec![(0., 0., 0.3), (9., 5., 0.3), (18., 10., 0.2)],
        },
        Rover {
            id: 2,
            x: 42.,
            y: -24.,
            yaw: -0.4,
            requested: "assisted_teleop",
            effective: "assisted_teleop",
            safety: "clear",
            revision: 3,
            goal: Some(goal(42., -24., 60., -8.)),
            route: vec![(42., -24., 0.2), (60., -8., 0.2)],
        },
        Rover {
            id: 3,
            x: -38.,
            y: 30.,
            yaw: 1.1,
            requested: "waypoint_direct",
            effective: "waypoint_direct",
            safety: "clear",
            revision: 3,
            goal: Some(goal(-38., 30., -18., 46.)),
            route: vec![(-38., 30., 0.25), (-18., 46., 0.25)],
        },
    ]
}

fn goal(x0: f64, y0: f64, x: f64, y: f64) -> Goal {
    Goal {
        x,
        y,
        state: "active",
        distance: (x - x0).hypot(y - y0),
    }
}

fn apply_commands(rx: &Receiver<(String, Vec<u8>)>, rovers: &mut [Rover]) {
    while let Ok((key, bytes)) = rx.try_recv() {
        let Some((id, kind)) = command_key(&key) else {
            continue;
        };
        let Some(rover) = rovers.iter_mut().find(|rover| rover.id == id) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        match kind {
            "autonomy" => {
                if let Some(level) = value["level"].as_str() {
                    rover.requested = leak(level);
                    rover.effective = leak(level);
                    rover.revision = rover.revision.saturating_add(1);
                    println!("fixture rover {id} level {level}");
                }
            }
            "safety" => match value["action"].as_str() {
                Some("stop") => {
                    rover.safety = "emergency_stop";
                    rover.revision = rover.revision.saturating_add(1);
                    println!("fixture rover {id} stop");
                }
                Some("reset") => {
                    rover.safety = "clear";
                    rover.revision = rover.revision.saturating_add(1);
                    println!("fixture rover {id} reset");
                }
                _ => {}
            },
            "goal" => {
                if value["cancel"] == true {
                    rover.goal = None;
                    println!("fixture rover {id} goal cancel");
                } else if let (Some(x), Some(y)) = (value["x"].as_f64(), value["y"].as_f64()) {
                    rover.goal = Some(goal(rover.x, rover.y, x, y));
                    println!("fixture rover {id} goal {x:.1},{y:.1}");
                }
            }
            _ => {}
        }
    }
}

fn command_key(key: &str) -> Option<(u64, &str)> {
    let rest = key.strip_prefix("terra/rover/")?;
    let (id, kind) = rest.split_once('/')?;
    if kind.contains('/') {
        return None;
    }
    Some((id.parse().ok()?, kind))
}

fn leak(level: &str) -> &'static str {
    match level {
        "teleop" => "teleop",
        "assisted_teleop" => "assisted_teleop",
        "waypoint_direct" => "waypoint_direct",
        "waypoint" => "waypoint",
        "explore" => "explore",
        "supervised" => "supervised",
        "target_search" => "target_search",
        "task" => "task",
        _ => "waypoint",
    }
}

fn publish(session: &zenoh::Session, rovers: &[Rover], tick: u64) {
    let ids: Vec<_> = rovers.iter().map(|rover| rover.id).collect();
    put(
        session,
        "terra/rover/fleet/state",
        json!({"count": ids.len(), "max_count": 32, "ids": ids}),
    );
    for rover in rovers {
        let id = rover.id;
        put(
            session,
            &format!("terra/rover/{id}/pose"),
            json!({
                "x": rover.x,
                "y": rover.y,
                "yaw": rover.yaw,
                "heading": rover.yaw,
                "sequence": tick,
            }),
        );
        put(
            session,
            &format!("terra/rover/{id}/hardware/status"),
            json!({
                "schema_version": 1,
                "control_mode": "streamed_relay",
                "ready": true,
                "connected": true,
                "output_active": false,
                "fault": null,
                "reason": "Fixture relay ready",
            }),
        );
        put(
            session,
            &format!("terra/rover/{id}/pose/quality"),
            json!({
                "schema_version": 1,
                "rover_id": id,
                "run_id": "fixture-run",
                "sequence": tick,
                "degraded": false,
                "reason": "",
                "pose_age_s": 0.1,
                "imu_age_s": 0.1,
            }),
        );
        let route: Vec<_> = rover
            .route
            .iter()
            .map(|(x, y, speed)| json!([x, y, speed]))
            .collect();
        put(
            session,
            &format!("terra/rover/{id}/autonomy/status"),
            json!({
                "requested_level": rover.requested,
                "effective_level": rover.effective,
                "assigned_level": rover.effective,
                "supported_levels": [
                    "teleop",
                    "assisted_teleop",
                    "waypoint_direct",
                    "waypoint",
                    "explore",
                    "supervised",
                    "target_search",
                    "task"
                ],
                "safety": rover.safety,
                "run_id": "fixture-run",
                "revision": rover.revision,
                "reason": "idle",
                "result": "accepted",
                "global_route": route,
            }),
        );
        let goal = match &rover.goal {
            Some(goal) => json!({
                "state": goal.state,
                "goal_id": id,
                "distance": goal.distance,
                "x": goal.x,
                "y": goal.y,
                "token": format!("goal-{id}"),
            }),
            None => json!({
                "state": "idle",
                "goal_id": 0,
                "distance": 0.0,
                "x": 0.0,
                "y": 0.0,
            }),
        };
        put(session, &format!("terra/rover/{id}/goal/status"), goal);
        if tick % 5 == 0 {
            put(
                session,
                &format!("terra/rover/{id}/map/occupancy"),
                occupancy(rover, tick),
            );
        }
    }
}

fn occupancy(rover: &Rover, tick: u64) -> Value {
    let width = 24u32;
    let height = 24u32;
    let mut cells = vec![0i8; (width * height) as usize];
    for cell in [40, 41, 64, 88, 89, 90] {
        cells[cell] = 90;
    }
    json!({
        "schema_version": 1,
        "rover_id": rover.id,
        "run_id": "fixture-run",
        "sequence": tick,
        "width": width,
        "height": height,
        "resolution": 0.5,
        "origin_x": rover.x - 6.0,
        "origin_y": rover.y - 6.0,
        "occupancy": cells,
    })
}

fn put(session: &zenoh::Session, key: &str, value: Value) {
    if let Err(error) = session.put(key, value.to_string()).wait() {
        eprintln!("fixture put {key}: {error}");
    }
}
