use argos_core::GoalRequest;
use argos_zenoh::Client;
use std::time::{Duration, Instant};
use zenoh::Wait;
fn wait_for(mut predicate: impl FnMut() -> bool) {
    let start = Instant::now();
    while !predicate() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "telemetry timed out"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[test]
fn receives_fleet_and_publishes_one_correlated_goal() {
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let endpoint = format!("tcp/127.0.0.1:{port}");
    let mut config = zenoh::Config::default();
    config
        .insert_json5("listen/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    config
        .insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    let peer = zenoh::open(config).wait().unwrap();
    let received = peer
        .declare_subscriber("terra/rover/8/goal")
        .wait()
        .unwrap();
    let client = Client::connect(&endpoint, "terra/rover").unwrap();
    wait_for(|| {
        peer.put(
            "terra/rover/fleet/state",
            br#"{"count":1,"max_count":32,"ids":[8]}"#.as_slice(),
        )
        .wait()
        .unwrap();
        client.snapshot().link == "healthy"
    });
    let grid = serde_json::json!({"schema_version":1,"rover_id":8,"run_id":"map-run","sequence":2,"width":2,"height":2,"resolution":0.5,"origin_x":-1,"origin_y":3,"occupancy":[-1,0,65,100]});
    wait_for(|| {
        peer.put("terra/rover/8/map/occupancy", grid.to_string())
            .wait()
            .unwrap();
        client.occupancy_snapshot().contains("map-run")
    });
    let observed: serde_json::Value = serde_json::from_str(&client.occupancy_snapshot()).unwrap();
    assert_eq!(observed["8"]["grid"]["occupancy"], grid["occupancy"]);
    let mut stale = grid.clone();
    stale["sequence"] = 1.into();
    stale["occupancy"] = serde_json::json!([0, 0, 0, 0]);
    peer.put("terra/rover/8/map/occupancy", stale.to_string())
        .wait()
        .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let observed: serde_json::Value = serde_json::from_str(&client.occupancy_snapshot()).unwrap();
    assert_eq!(observed["8"]["grid"]["sequence"], 2);
    let token = client
        .send_goal(
            8,
            GoalRequest::Local {
                x: 12.,
                y: 0.,
                yaw: None,
            },
        )
        .unwrap();
    let sample = received
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&sample.payload().to_bytes()).unwrap();
    assert_eq!(body["token"], token);
    assert!(
        received
            .recv_timeout(Duration::from_millis(100))
            .unwrap()
            .is_none()
    );
    peer.put("terra/rover/8/goal/status", format!("{{\"state\":\"arrived\",\"goal_id\":1,\"distance\":0,\"x\":12,\"y\":0,\"token\":\"{token}\"}}")).wait().unwrap();
    wait_for(|| client.snapshot().rovers[0].command_phase == "arrived");
    client.disconnect();
    client.disconnect();
    assert_eq!(client.snapshot().link, "disconnected");
}
#[test]
fn authority_teleop_release_and_export_are_correlated() {
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("tcp/127.0.0.1:{}", socket.local_addr().unwrap().port());
    drop(socket);
    let mut config = zenoh::Config::default();
    config
        .insert_json5(
            "listen/endpoints",
            &serde_json::json!([endpoint]).to_string(),
        )
        .unwrap();
    config
        .insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    let peer = zenoh::open(config).wait().unwrap();
    let tele = peer
        .declare_subscriber("terra/rover/8/teleop")
        .wait()
        .unwrap();
    let actions = peer
        .declare_subscriber("terra/rover/8/autonomy")
        .wait()
        .unwrap();
    let c = Client::connect(&endpoint, "terra/rover").unwrap();
    wait_for(|| {
        peer.put(
            "terra/rover/fleet/state",
            r#"{"count":1,"max_count":32,"ids":[8]}"#,
        )
        .wait()
        .unwrap();
        c.snapshot().link == "healthy"
    });
    let token = c
        .action(8, "autonomy", serde_json::json!({"level":"teleop"}))
        .unwrap();
    assert!(
        actions
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .is_some()
    );
    wait_for(|| {
        peer.put("terra/rover/8/autonomy/status",serde_json::json!({"requested_level":"teleop","effective_level":"teleop","active_source":"none","safety":"clear","reason":"idle","revision":1,"run_id":"run","token":token,"result":"accepted","supported_levels":["teleop"],"paused":false,"assigned_level":null}).to_string()).wait().unwrap();
        c.operator_snapshot().contains("accepted")
    });
    c.action(8, "mission/report", serde_json::json!({"survivor_id":1}))
        .unwrap();
    // Observational reporting does not create an authority gate without an authority receipt.
    c.input(8, "w", true).unwrap();
    let sample = tele.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&sample.payload().to_bytes()).unwrap();
    assert_eq!(v["linear"], 1.);
    let initial_sequence = v["sequence"].as_u64().unwrap();
    c.clear_input_event(10);
    c.input_event(8, "w", true, 9).unwrap();
    c.input_generation(8, "w", true, 11, "previous-run", 1)
        .unwrap();
    let mut zero = false;
    for _ in 0..5 {
        if let Some(sample) = tele.recv_timeout(Duration::from_millis(100)).unwrap() {
            let v: serde_json::Value =
                serde_json::from_slice(&sample.payload().to_bytes()).unwrap();
            if v["linear"] == 0 {
                assert!(v["sequence"].as_u64().unwrap() > initial_sequence);
                zero = true;
            } else {
                panic!("stale input rearmed: {v}");
            }
        }
    }
    assert!(zero);
    let p = std::env::temp_dir().join(format!("argos-export-{}.jsonl", uuid::Uuid::new_v4()));
    c.export_session(p.to_str().unwrap()).unwrap();
    let exported = std::fs::read_to_string(&p).unwrap();
    assert!(exported.contains(&token));
    assert!(exported.contains("teleop_sent"));
    std::fs::remove_file(p).unwrap();
    c.disconnect();
}
