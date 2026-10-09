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
    let operator: serde_json::Value = serde_json::from_str(&client.operator_snapshot()).unwrap();
    assert_eq!(operator["8"]["occupancy"]["sequence"], 2);
    assert_eq!(
        operator["8"]["occupancy"]["occupancy"],
        serde_json::json!([-1, 0, 65, 100])
    );
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

#[test]
fn receives_phone_pose_without_depth_pixels() {
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("tcp/127.0.0.1:{}", socket.local_addr().unwrap().port());
    drop(socket);
    let mut config = zenoh::Config::default();
    config.insert_json5("mode", "\"router\"").unwrap();
    config
        .insert_json5("listen/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    config
        .insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    let router = zenoh::open(config).wait().unwrap();
    let client = Client::connect(&endpoint, "phone-test").unwrap();
    wait_for(|| {
        router
            .put(
                "phone-test/fleet/state",
                br#"{"count":1,"max_count":1,"ids":[7]}"#.as_slice(),
            )
            .wait()
            .unwrap();
        client.snapshot().link == "healthy"
    });
    wait_for(|| {
        router
            .put(
                "phone-test/7/pose",
                br#"{"rover_id":7,"sequence":1,"x":2,"y":-3,"yaw":0.5}"#.as_slice(),
            )
            .wait()
            .unwrap();
        client.snapshot().rovers[0].pose.is_some()
    });
    assert_eq!(client.snapshot().rovers[0].pose.as_ref().unwrap().y, -3.);
    wait_for(|| {
        router
            .put(
                "phone-test/7/localization",
                br#"{"version":1,"mode":"geographic","frameID":"session"}"#.as_slice(),
            )
            .wait()
            .unwrap();
        client.localization_snapshot().contains("session")
    });
    let localization: serde_json::Value =
        serde_json::from_str(&client.localization_snapshot()).unwrap();
    assert!(localization["7"]["age"].as_f64().unwrap() < 2.5);
    // A different rover ID cannot overwrite the row identified by the topic.
    router
        .put(
            "phone-test/7/pose",
            br#"{"rover_id":8,"sequence":2,"x":99,"y":0,"yaw":0}"#.as_slice(),
        )
        .wait()
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(client.snapshot().rovers[0].pose.as_ref().unwrap().x, 2.);
    let commands = router
        .declare_subscriber("phone-test/7/teleop")
        .wait()
        .unwrap();
    wait_for(|| {
        router
            .put(
                "phone-test/7/pose",
                br#"{"rover_id":7,"sequence":3,"x":2,"y":-3,"yaw":0.5}"#.as_slice(),
            )
            .wait()
            .unwrap();
        router.put("phone-test/7/autonomy/status", br#"{"run_id":"run","revision":4,"requested_level":"teleop","effective_level":"teleop","safety":"clear","supported_levels":["teleop","waypoint"]}"#.as_slice()).wait().unwrap();
        router.put("phone-test/7/map/occupancy", br#"{"schema_version":1,"rover_id":7,"run_id":"run","sequence":1,"width":1,"height":1,"resolution":1,"origin_x":0,"origin_y":0,"occupancy":[0]}"#.as_slice()).wait().unwrap();
        router
            .put(
                "phone-test/7/hardware/status",
                br#"{"ready":true,"armed":true,"arming":false,"reason":"Armed"}"#.as_slice(),
            )
            .wait()
            .unwrap();
        client.operator_snapshot().contains("occupancy")
            && client.operator_snapshot().contains("hardware")
    });
    let hardware_commands = router
        .declare_subscriber("phone-test/7/hardware")
        .wait()
        .unwrap();
    assert!(
        client
            .operator_command(
                7,
                "hardware",
                r#"{"action":"arm","token":"bad","run_id":"other","authority_revision":4}"#
            )
            .is_err()
    );
    client
        .operator_command(
            7,
            "hardware",
            r#"{"action":"arm","token":"arm-one","run_id":"run","authority_revision":4}"#,
        )
        .unwrap();
    let sample = hardware_commands
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let arm: serde_json::Value = serde_json::from_slice(&sample.payload().to_bytes()).unwrap();
    assert_eq!(arm["action"], "arm");
    assert!(
        client
            .operator_command(7, "teleop", r#"{"linear":1,"angular":0}"#)
            .is_err()
    );
    assert!(
        client
            .operator_command(7, "teleop", r#"{"linear":0.5,"angular":0}"#)
            .is_err()
    );
    client.operator_command(7, "teleop", r#"{"linear":0.5,"angular":0,"run_id":"run","authority_revision":4,"operator_session_id":"session","sequence":1}"#).unwrap();
    let sample = commands
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let command: serde_json::Value = serde_json::from_slice(&sample.payload().to_bytes()).unwrap();
    assert_eq!(command["linear"], 0.5);
    assert_eq!(command["authority_revision"], 4);
    std::thread::sleep(Duration::from_millis(600));
    client
        .operator_command(7, "hardware", r#"{"action":"disarm","token":"stop-one"}"#)
        .unwrap();
    assert!(
        hardware_commands
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .is_some()
    );
    assert!(client.operator_command(7, "teleop", r#"{"linear":0.5,"angular":0,"run_id":"run","authority_revision":4,"operator_session_id":"session","sequence":2}"#).is_err());
    client.operator_command(7, "teleop", r#"{"linear":0,"angular":0,"run_id":"run","authority_revision":4,"operator_session_id":"session","sequence":3}"#).unwrap();
    let sample = commands
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let command: serde_json::Value = serde_json::from_slice(&sample.payload().to_bytes()).unwrap();
    assert_eq!(command["linear"], 0);
    std::thread::sleep(Duration::from_millis(2600));
    assert_eq!(client.localization_snapshot(), "{}");
    client.disconnect();
    assert_eq!(client.localization_snapshot(), "{}");
}

#[test]
fn search_reports_are_retained_and_acknowledged_on_canonical_topics() {
    use std::time::{Duration, Instant};
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
    let server = zenoh::open(config).wait().unwrap();
    let ack = server
        .declare_subscriber("search/7/search/report/ack")
        .wait()
        .unwrap();
    let client = argos_zenoh::Client::connect(&endpoint, "search").unwrap();
    let start = Instant::now();
    loop {
        for id in ["first", "second"] {
            server.put("search/7/search/report",serde_json::json!({"version":1,"run_id":"run","search_id":id,"report_id":format!("{id}:report"),"target_class":"survivor","world_x":1,"world_y":2,"frame_ids":[1,2,3],"evidence_ids":["a","b","c"]}).to_string()).wait().unwrap();
        }
        let v: serde_json::Value = serde_json::from_str(&client.operator_snapshot()).unwrap();
        if v["7"]["reports"].as_array().is_some_and(|r| r.len() == 2) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(20));
    }
    client.operator_command(7,"search/report/ack",r#"{"version":1,"run_id":"run","search_id":"first","report_id":"first:report","token":"ack"}"#).unwrap();
    loop {
        if ack.try_recv().unwrap().is_some() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(10));
    }
    let log_path =
        std::env::temp_dir().join(format!("l4-report-export-{}.jsonl", std::process::id()));
    client.export_session(log_path.to_str().unwrap()).unwrap();
    let records: Vec<serde_json::Value> = std::fs::read_to_string(&log_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(records.iter().any(|r| r["kind"] == "action_sent"
        && r["topic"] == "search/report/ack"
        && r["payload"]["report_id"] == "first:report"));
    assert!(records.iter().any(|r| r["kind"] == "state_observed"
        && r["topic"] == "search/report"
        && r["payload"]["search_id"] == "second"));
    std::fs::remove_file(log_path).unwrap();
    client.disconnect();
    server.close().wait().unwrap();
}
