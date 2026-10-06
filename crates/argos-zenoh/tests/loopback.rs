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
