use argos_core::GoalRequest;
use argos_zenoh::Client;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
fn wait(c: &Client, id: u64, mut check: impl FnMut(&Value) -> bool) -> Value {
    let start = Instant::now();
    loop {
        let states: Value = serde_json::from_str(&c.operator_snapshot()).unwrap();
        let s = &states[id.to_string()];
        if check(s) {
            return s.clone();
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "state timeout: {s}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tcp/127.0.0.1:7448".into());
    let c = Client::connect(&endpoint, "terra/rover")?;
    let start = Instant::now();
    while c.snapshot().link != "healthy" {
        assert!(start.elapsed().as_secs() < 10);
        std::thread::sleep(Duration::from_millis(100));
    }
    let ids = c
        .snapshot()
        .rovers
        .iter()
        .filter(|r| r.membership == "online")
        .map(|r| r.id)
        .collect::<Vec<_>>();
    let id = ids[0];
    let mut evidence = Vec::new();
    for level in ["teleop", "assisted_teleop", "waypoint", "supervised"] {
        let at = Instant::now();
        let token = c.action(id, "autonomy", json!({"level":level}))?;
        let s = wait(&c, id, |s| {
            s["status"]["token"] == token
                && s["status"]["result"] == "accepted"
                && s["status"]["safety"] == "clear"
        });
        evidence.push(json!({"level":level,"ack_latency":at.elapsed().as_secs_f64(),"state":s}));
        if level == "teleop" || level == "assisted_teleop" {
            c.input(id, "w", true)?;
            std::thread::sleep(Duration::from_millis(800));
            c.clear_input();
        }
        if level == "waypoint" {
            let token = c.send_goal(
                id,
                GoalRequest::Local {
                    x: c.snapshot()
                        .rovers
                        .iter()
                        .find(|r| r.id == id)
                        .unwrap()
                        .pose
                        .as_ref()
                        .unwrap()
                        .x
                        + 2.,
                    y: 0.,
                    yaw: None,
                },
            )?;
            let start = Instant::now();
            loop {
                let snapshot = c.snapshot();
                let r = snapshot.rovers.iter().find(|r| r.id == id).unwrap();
                if r.command_phase == "arrived" {
                    evidence.push(json!({"goal_token":token,"phase":"arrived","distance":r.goal.as_ref().unwrap().distance}));
                    break;
                }
                assert!(start.elapsed().as_secs() < 30, "goal failed: {r:?}");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        if level == "supervised" {
            let s = wait(&c, id, |s| s["proposal"]["proposal_id"].as_u64().is_some());
            let proposal = s["proposal"]["proposal_id"].as_u64().unwrap();
            let token=c.action(id,"goal/decision",json!({"decision":"approve","proposal_id":proposal,"run_id":s["proposal"]["run_id"]}))?;
            let approved = wait(&c, id, |s| {
                s["status"]["token"] == token && s["status"]["result"] == "accepted"
            });
            std::thread::sleep(Duration::from_secs(2));
            evidence.push(json!({"proposal_id":proposal,"decision":approved}));
        }
    }
    let takeover = c.fleet_action("autonomy", json!({"level":"teleop"}));
    for (id, token) in &takeover {
        wait(&c, *id, |s| {
            s["status"]["token"] == *token && s["status"]["requested_level"] == "teleop"
        });
    }
    let stopped = c.fleet_action("safety", json!({"action":"stop"}));
    for (id, token) in &stopped {
        wait(&c, *id, |s| {
            s["status"]["token"] == *token && s["status"]["safety"] == "emergency_stop"
        });
    }
    evidence.push(json!({"fleet_takeover":takeover,"fleet_stop":stopped}));
    let path = "/private/tmp/argos-mission-operator-session.jsonl";
    c.export_session(path)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"evidence":evidence,"operator_log":path}))?
    );
    c.disconnect();
    Ok(())
}
