//! Loopback Zenoh router for dashboard screenshots.
//!
//! Listens on `tcp/127.0.0.1:7447` with prefix `terra/rover` (the `--sim`
//! endpoint) and publishes three rovers. Operator commands on `autonomy`,
//! `safety`, and `goal` are applied on the next tick so Take over, Drive,
//! and Stop can be captured from the live window.
//!
//! `ARGOS_FIXTURE_DRIVE=1` also plays goals and docks out, for the Mission
//! tab: rovers drive to their goal and report `arrived`, rover 1 has found an
//! orange bucket, and a `dock` start lines up, waits for the dump approval,
//! then reports `done`.
use serde_json::{Value, json};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};
use zenoh::Wait;

struct Goal {
    /// Echoed on goal/status so ARGOS sees its goal acknowledged.
    token: String,
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
    /// Dock id, bucket, phase, ticks in that phase.
    dock: Option<(u64, u32, &'static str, u32)>,
}

/// Drive demo speed, metres per 200 ms tick.
const STEP_M: f64 = 3.;
/// Rover 1's found bucket in the drive demo: id, x, y.
const BUCKET: (u32, f64, f64) = (4, 14., 10.);

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
    for kind in ["autonomy", "safety", "goal", "dock"] {
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
    let drive = std::env::var("ARGOS_FIXTURE_DRIVE").as_deref() == Ok("1");
    if drive {
        for rover in &mut rovers {
            rover.goal = None;
            rover.route.clear();
        }
    }
    loop {
        apply_commands(&rx, &mut rovers);
        if drive {
            for rover in &mut rovers {
                advance(rover);
            }
            put(
                &session,
                "terra/rover/1/objects",
                json!({"schema_version": 1, "objects": [{
                    "id": BUCKET.0, "class": "bucket", "color": "orange",
                    "x": BUCKET.1, "y": BUCKET.2, "confirmed": true,
                    "confidence": 0.9, "sightings": 12,
                }]}),
            );
        }
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
            dock: None,
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
            dock: None,
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
            dock: None,
            route: vec![(-38., 30., 0.25), (-18., 46., 0.25)],
        },
    ]
}

/// Drive demo: 3 m per tick toward an active goal, then `arrived`; a dock
/// steps lock → aligned (awaits the dump) and dumping → done.
fn advance(rover: &mut Rover) {
    if rover.safety != "clear" {
        return;
    }
    if let Some(goal) = rover.goal.as_mut().filter(|goal| goal.state == "active") {
        let (dx, dy) = (goal.x - rover.x, goal.y - rover.y);
        let distance = dx.hypot(dy);
        if distance <= STEP_M {
            (rover.x, rover.y) = (goal.x, goal.y);
            goal.state = "arrived";
            goal.distance = 0.;
        } else {
            rover.x += dx / distance * STEP_M;
            rover.y += dy / distance * STEP_M;
            rover.yaw = dy.atan2(dx);
            goal.distance = distance - STEP_M;
        }
    }
    if let Some((id, bucket, phase, ticks)) = rover.dock {
        rover.dock = Some(match (phase, ticks) {
            ("lock", 5) => (id, bucket, "aligned", 0),
            ("dumping", 5) => (id, bucket, "done", 0),
            _ => (id, bucket, phase, ticks + 1),
        });
    }
}

fn goal(x0: f64, y0: f64, x: f64, y: f64) -> Goal {
    Goal {
        token: "fixture".into(),
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
                    // As Terra: a goal cancel drops L2/L3 to manual.
                    if matches!(rover.effective, "waypoint" | "waypoint_direct") {
                        rover.requested = "teleop";
                        rover.effective = "teleop";
                        rover.revision = rover.revision.saturating_add(1);
                    }
                    println!("fixture rover {id} goal cancel");
                } else if let (Some(x), Some(y)) = (value["x"].as_f64(), value["y"].as_f64()) {
                    let mut next = goal(rover.x, rover.y, x, y);
                    if let Some(token) = value["token"].as_str() {
                        next.token = token.into();
                    }
                    rover.goal = Some(next);
                    println!("fixture rover {id} goal {x:.1},{y:.1}");
                }
            }
            "dock" => match value["action"].as_str() {
                Some("start") => {
                    let bucket = value["object_id"].as_u64().unwrap_or(0) as u32;
                    let next = rover.dock.map_or(1, |(id, ..)| id + 1);
                    rover.dock = Some((next, bucket, "lock", 0));
                    println!("fixture rover {id} dock start {bucket}");
                }
                Some("dump") => {
                    if let Some((dock, bucket, "aligned", _)) = rover.dock {
                        rover.dock = Some((dock, bucket, "dumping", 0));
                        println!("fixture rover {id} dump");
                    }
                }
                Some("cancel") => {
                    if let Some((dock, bucket, _, _)) = rover.dock {
                        rover.dock = Some((dock, bucket, "cancelled", 0));
                    }
                }
                _ => {}
            },
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
                "dock": rover.dock.map(|(dock_id, bucket, phase, _)| json!({
                    "dock_id": dock_id,
                    "phase": phase,
                    "reason": "",
                    "object_id": bucket,
                    "color": "orange",
                    "errors": {"lateral": 0.01, "range": 0.0, "heading_deg": 2.0},
                    "in_tolerance": phase == "aligned",
                    "verified": true,
                    "approval": "operator",
                    "awaiting_approval": phase == "aligned",
                    "gate_open": phase == "dumping",
                })),
            }),
        );
        let goal = match &rover.goal {
            Some(goal) => json!({
                "state": goal.state,
                "goal_id": id,
                "distance": goal.distance,
                "x": goal.x,
                "y": goal.y,
                "token": goal.token,
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
