use super::*;
use std::time::Duration;
#[test]
fn snapshots_preserve_large_grids_membership_and_freshness() {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let router = router::Router::default();
    router.start(port, "").unwrap();
    let mut client = Client::connect(&format!("tcp/127.0.0.1:{port}"), "snapshot-test").unwrap();
    client.start = Instant::now() - Duration::from_secs(120);
    client
        .operator
        .lock()
        .unwrap()
        .pending(8, "token".into(), 0.);
    {
        let mut cache = client.cache.lock().unwrap();
        cache.fleet(
            decode_fleet(br#"{"count":2,"max_count":32,"ids":[8,9]}"#).unwrap(),
            0.,
        );
        cache.fleet(
            decode_fleet(br#"{"count":1,"max_count":32,"ids":[8]}"#).unwrap(),
            0.,
        );
    }
    let grid = OccupancyGrid {
        schema_version: 1,
        rover_id: 8,
        run_id: "snapshot-run".into(),
        sequence: 42,
        width: 500,
        height: 500,
        resolution: 0.05,
        origin_x: -12.5,
        origin_y: 3.,
        occupancy: (0..250_000).map(|i| (i % 102) as i8 - 1).collect(),
    };
    let expected = serde_json::to_value(&grid).unwrap();
    {
        let mut maps = client.maps.lock().unwrap();
        maps.insert(8, (grid.clone(), 10_000., None));
        maps.insert(9, (grid.clone(), 0., None));
        maps.insert(10, (grid, 0., None));
    }
    let now = Instant::now();
    {
        let mut states = client.operator_states.lock().unwrap();
        states.insert(
            (8, "hardware".into()),
            (
                serde_json::json!({"ok": true}),
                now - Duration::from_secs(3),
            ),
        );
        states.insert(
            (8, "search_report:first".into()),
            (serde_json::json!({"sequence": 7}), now),
        );
        states.insert(
            (8, "search_report:expired".into()),
            (
                serde_json::json!({"sequence": 6}),
                now - Duration::from_secs(3),
            ),
        );
        states.insert(
            (8, "authority".into()),
            (
                serde_json::json!({"revision": 1}),
                now - Duration::from_secs_f64(SUPERVISION_STALE_SECONDS + 1.),
            ),
        );
    }
    let occupancy: serde_json::Value = serde_json::from_str(&client.occupancy_snapshot()).unwrap();
    assert_eq!(
        occupancy,
        serde_json::json!({"8": {"grid": expected, "age": 0.}})
    );
    let operator: serde_json::Value = serde_json::from_str(&client.operator_snapshot()).unwrap();
    let mut embedded = expected.clone();
    embedded["age"] = serde_json::json!(0.);
    assert_eq!(operator["8"]["occupancy"], embedded);
    assert!(operator.get("9").is_none());
    assert!(operator.get("10").is_none());
    assert_eq!(operator["8"]["hardware"]["ok"], true);
    assert!(operator["8"]["hardware"]["age"].as_f64().unwrap() >= 0.);
    assert_eq!(operator["8"]["reports"].as_array().unwrap().len(), 1);
    assert_eq!(operator["8"]["reports"][0]["sequence"], 7);
    assert_eq!(
        operator["8"]["authority"],
        client.legacy_operator_snapshot_value()["8"]["authority"]
    );
    assert_eq!(operator["8"]["recording"], "active");
    client.disconnect();
    router.stop().unwrap();
}

#[test]
fn waypoint_mode_accepts_delayed_reports_but_manual_mode_does_not() {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let router = router::Router::default();
    router.start(port, "").unwrap();
    let mut client = Client::connect(&format!("tcp/127.0.0.1:{port}"), "forest-test").unwrap();
    client.start = Instant::now() - Duration::from_secs(60);
    {
        let mut cache = client.cache.lock().unwrap();
        cache.fleet(
            decode_fleet(br#"{"count":1,"max_count":32,"ids":[8]}"#).unwrap(),
            0.,
        );
        cache.pose(
            8,
            Pose {
                rover_id: 8,
                sequence: 1,
                x: 0.,
                y: 0.,
                yaw: 0.,
            },
            0.,
        );
    }
    client.operator_states.lock().unwrap().insert(
        (8, "authority".into()),
        (
            serde_json::json!({
                "run_id": "run",
                "revision": 1,
                "requested_level": "teleop",
                "supported_levels": ["teleop","waypoint","waypoint_direct"],
                "safety": "clear",
            }),
            Instant::now() - Duration::from_secs(60),
        ),
    );
    assert!(
        client
            .operator_command(8, "autonomy", r#"{"level":"waypoint","token":"delayed"}"#)
            .is_ok()
    );
    assert!(
        client
            .operator_command(
                8,
                "autonomy",
                r#"{"level":"waypoint_direct","token":"delayed-direct"}"#
            )
            .is_ok()
    );
    assert!(
        client
            .operator_command(8, "autonomy", r#"{"level":"teleop","token":"manual"}"#)
            .is_err()
    );
    // Camera pose can be old while control authority and membership are fresh.
    client.cache.lock().unwrap().fleet(
        decode_fleet(br#"{"count":1,"max_count":32,"ids":[8]}"#).unwrap(),
        client.start.elapsed().as_secs_f64(),
    );
    client
        .operator_states
        .lock()
        .unwrap()
        .get_mut(&(8, "authority".into()))
        .unwrap()
        .1 = Instant::now();
    assert!(
        client
            .operator_command(
                8,
                "autonomy",
                r#"{"level":"teleop","token":"manual-no-camera"}"#
            )
            .is_ok()
    );
    assert!(
        client
            .operator_command(
                8,
                "hardware",
                r#"{"action":"arm","token":"manual-arm","run_id":"run","authority_revision":1}"#
            )
            .is_ok()
    );
    assert!(
        client
            .operator_command(
                8,
                "autonomy",
                r#"{"level":"assisted_teleop","token":"assisted-no-camera"}"#
            )
            .is_err()
    );
    client.disconnect();
    router.stop().unwrap();
}
