//! Real local Zenoh transport; no hardware, Apple runtime, or external network.
use argos_server::{
    engine::{Engine, Fleet},
    fleet::LiveFleet,
    types::*,
};
use std::time::{Duration, Instant};
use zenoh::Wait;
#[test]
fn live_adapter_observes_local_frames_and_publishes_one_correlated_waypoint() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("tcp/127.0.0.1:{}", listener.local_addr().unwrap().port());
    drop(listener);
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
    let goals = peer.declare_subscriber("web-test/8/goal").wait().unwrap();
    let modes = peer
        .declare_subscriber("web-test/8/autonomy")
        .wait()
        .unwrap();
    let teleop = peer.declare_subscriber("web-test/8/teleop").wait().unwrap();
    let safety = peer.declare_subscriber("web-test/8/safety").wait().unwrap();
    let hardware = peer
        .declare_subscriber("web-test/8/hardware")
        .wait()
        .unwrap();
    let mut e = Engine::new(LiveFleet::new(endpoint, "web-test".into()));
    let start = Instant::now();
    let mut authority = serde_json::json!({"run_id":"test-run","revision":1,"requested_level":"teleop","effective_level":"teleop","active_source":"none","safety":"clear","reason":"test","supported_levels":["waypoint","teleop"],"token":"initial","result":"accepted"});
    loop {
        let _ = e.fleet.snapshot(start.elapsed().as_secs_f64());
        for (topic, value) in [
            (
                "fleet/state",
                serde_json::json!({"count":1,"max_count":32,"ids":[8]}),
            ),
            (
                "8/pose",
                serde_json::json!({"rover_id":8,"sequence":1,"x":0.,"y":0.,"yaw":0.}),
            ),
            (
                "8/localization",
                serde_json::json!({"version":1,"frameID":"local-A","mode":"local","tracking":"limited","gpsLatitude":38.8,"gpsLongitude":-77.3,"gpsAccuracy":24.0,"gpsFixAge":0.1,"headingAccuracy":-1}),
            ),
            (
                "8/hardware/status",
                serde_json::json!({"ready":true,"armed":true,"reason":"test"}),
            ),
            ("8/autonomy/status", authority.clone()),
            (
                "8/map/occupancy",
                serde_json::json!({"schema_version":1,"rover_id":8,"run_id":"test-run","sequence":9007199254740993_u64,"width":2,"height":2,"resolution":0.5,"origin_x":-1.,"origin_y":-2.,"occupancy":[-1,0,64,65]}),
            ),
        ] {
            peer.put(format!("web-test/{topic}"), value.to_string())
                .wait()
                .unwrap();
        }
        std::thread::sleep(Duration::from_millis(30));
        let snap = e.fleet.snapshot(start.elapsed().as_secs_f64());
        if snap
            .rovers
            .first()
            .is_some_and(|r| r.ready() && r.frame_id.as_deref() == Some("local-A"))
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "Live adapter did not receive fleet telemetry"
        );
    }
    assert!(
        e.snapshot(0.).rovers[0].reference.is_none(),
        "Local frames do not require geographic alignment"
    );
    let raw = serde_json::to_value(&e.snapshot(0.).rovers[0]).unwrap();
    assert_eq!(
        raw["gps"]["latitude"], 38.8,
        "raw GPS needs no AR tracking or heading alignment"
    );
    assert_eq!(
        raw["gps"]["accuracy"], 24.0,
        "low accuracy remains visible as a measurement"
    );
    let map = e.fleet.occupancy("8", 0.).unwrap().unwrap();
    assert_eq!(map.sequence, "9007199254740993");
    assert_eq!(map.cells, vec![-1, 0, 64, 65]);
    assert_eq!(map.frame_id.as_deref(), Some("local-A"));
    let user = Identity {
        id: "operator".into(),
        login: "test".into(),
        role: "operator".into(),
        csrf: String::new(),
    };
    e.claim(&user, "tab", 0.).unwrap();
    let mut drive = DriveRequest {
        run_id: "test-run".into(),
        revision: "1".into(),
        sequence: 1,
        observed_at: 0.,
        linear: 0.3,
        angular: 0.,
    };
    e.drive(&user, "tab", "8", drive.clone(), 0.).unwrap();
    let packet = teleop
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let moving: serde_json::Value = serde_json::from_slice(&packet.payload().to_bytes()).unwrap();
    assert_eq!(moving["linear"], 0.3);
    assert_eq!(moving["run_id"], "test-run");
    assert_eq!(moving["authority_revision"], 1);
    drive.sequence = 2;
    drive.linear = 0.;
    e.drive(&user, "tab", "8", drive, 0.01).unwrap();
    let packet = teleop
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let neutral: serde_json::Value = serde_json::from_slice(&packet.payload().to_bytes()).unwrap();
    assert_eq!(neutral["linear"], 0.);
    assert_eq!(
        neutral["operator_session_id"],
        moving["operator_session_id"]
    );
    assert!(neutral["sequence"].as_u64().unwrap() > moving["sequence"].as_u64().unwrap());
    let op = e
        .submit(
            &user,
            "tab",
            "8",
            WaypointRequest {
                run_id: "test-run".into(),
                frame_id: Some("local-A".into()),
                request_id: uuid::Uuid::new_v4().to_string(),
                waypoint: Waypoint::Local {
                    x: 12.,
                    y: 3.,
                    yaw: None,
                },
                arrival_radius: None,
            },
            0.,
        )
        .unwrap();
    let mode = modes.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
    let mode: serde_json::Value = serde_json::from_slice(&mode.payload().to_bytes()).unwrap();
    authority["requested_level"] = "waypoint".into();
    authority["effective_level"] = "waypoint".into();
    authority["token"] = mode["token"].clone();
    loop {
        peer.put("web-test/8/autonomy/status", authority.to_string())
            .wait()
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        e.tick(start.elapsed().as_secs_f64());
        if e.operations[&op.id].phase == "published" {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
    }
    let goal = goals.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
    let goal: serde_json::Value = serde_json::from_slice(&goal.payload().to_bytes()).unwrap();
    assert_eq!(goal["x"], 12.);
    assert_eq!(
        goal["token"],
        e.operations[&op.id].goal_token.clone().unwrap()
    );
    e.tick(start.elapsed().as_secs_f64());
    assert!(
        goals
            .recv_timeout(Duration::from_millis(100))
            .unwrap()
            .is_none()
    );
    peer.put(
        "web-test/8/localization",
        serde_json::json!({"version":1,"frameID":"local-B","mode":"local","tracking":"normal"})
            .to_string(),
    )
    .wait()
    .unwrap();
    let frame_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if e.fleet.snapshot(0.).rovers[0].frame_id.as_deref() == Some("local-B") {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
        assert!(Instant::now() < frame_deadline);
    }
    assert!(
        e.fleet.occupancy("8", 0.).unwrap().is_none(),
        "Old frame cells must be invalidated even in the same authority run"
    );
    peer.put("web-test/8/map/occupancy",serde_json::json!({"schema_version":1,"rover_id":8,"run_id":"test-run","sequence":9007199254740994_u64,"width":1,"height":1,"resolution":0.5,"origin_x":0.,"origin_y":0.,"occupancy":[65]}).to_string()).wait().unwrap();
    loop {
        if let Some(map) = e.fleet.occupancy("8", 0.).unwrap() {
            assert_eq!(map.frame_id.as_deref(), Some("local-B"));
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
        assert!(Instant::now() < frame_deadline);
    }
    authority["run_id"] = "new-run".into();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        peer.put("web-test/8/autonomy/status", authority.to_string())
            .wait()
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        if e.fleet.snapshot(0.).rovers[0]
            .authority
            .as_ref()
            .unwrap()
            .run_id
            == "new-run"
        {
            break;
        }
        assert!(Instant::now() < deadline);
    }
    assert!(
        e.fleet.occupancy("8", 0.).unwrap().is_none(),
        "Previous-run grid must not be drawn in the new run"
    );
    e.emergency_stop(&user, "tab", "8", start.elapsed().as_secs_f64())
        .unwrap();
    let packet = safety
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let stop: serde_json::Value = serde_json::from_slice(&packet.payload().to_bytes()).unwrap();
    assert_eq!(stop["action"], "stop");
    assert!(stop["token"].as_str().is_some_and(|s| !s.is_empty()));
    let packet = hardware
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let disarm: serde_json::Value = serde_json::from_slice(&packet.payload().to_bytes()).unwrap();
    assert_eq!(disarm["action"], "disarm");
    assert!(e.claim.is_none());
}
