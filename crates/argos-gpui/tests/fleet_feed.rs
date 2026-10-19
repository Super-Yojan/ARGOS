//! Headless check that a Zorvane-shaped publisher moves the map's data layer.
use argos_core::{Anchor, LocalFrame};
use argos_gpui::{DashboardConfig, fleet_marks};
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
fn zorvane_pose_updates_marks_and_occupancy() {
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
    let client = Client::connect(&endpoint, "terra/rover").unwrap();
    let origin = LocalFrame::origin(0., 0.).unwrap();
    wait_for(|| {
        router
            .put(
                "terra/rover/fleet/state",
                br#"{"count":1,"max_count":1,"ids":[3]}"#.as_slice(),
            )
            .wait()
            .unwrap();
        client.snapshot().link == "healthy"
    });
    wait_for(|| {
        router
            .put(
                "terra/rover/3/pose",
                br#"{"x":0,"y":0,"yaw":0,"heading":0}"#.as_slice(),
            )
            .wait()
            .unwrap();
        let located = serde_json::Value::Null;
        !fleet_marks(&client.snapshot(), &located, |_| None, origin).is_empty()
    });
    let start = fleet_marks(
        &client.snapshot(),
        &serde_json::Value::Null,
        |_| None,
        origin,
    );
    assert_eq!(start[0].status, "online");
    assert!((start[0].heading).abs() < 1e-6);
    wait_for(|| {
        router
            .put(
                "terra/rover/3/pose",
                br#"{"x":20,"y":0,"yaw":1.5707963267948966,"heading":1.5707963267948966}"#
                    .as_slice(),
            )
            .wait()
            .unwrap();
        router
            .put(
                "terra/rover/3/map/occupancy",
                br#"{"schema_version":1,"rover_id":3,"run_id":"run","sequence":1,"width":2,"height":1,"resolution":0.5,"origin_x":0,"origin_y":0,"occupancy":[0,80]}"#.as_slice(),
            )
            .wait()
            .unwrap();
        let marks = fleet_marks(
            &client.snapshot(),
            &serde_json::Value::Null,
            |id| client.occupancy_grid(id).map(|(grid, age, _)| (grid, age)),
            origin,
        );
        marks
            .first()
            .is_some_and(|mark| mark.latitude > start[0].latitude + 1e-6)
            && marks
                .first()
                .is_some_and(|mark| mark.occupancy.contains("occupied"))
    });
    let moved = fleet_marks(
        &client.snapshot(),
        &serde_json::Value::Null,
        |id| client.occupancy_grid(id).map(|(grid, age, _)| (grid, age)),
        origin,
    );
    assert!((moved[0].heading - 270.).abs() < 1e-3);
    assert!(moved[0].occupancy.starts_with("2x1, 1 occupied"));

    let aligned = DashboardConfig::parse(None, None, None, None, (None, None), (None, None))
        .unwrap()
        .origin;
    wait_for(|| {
        router
            .put(
                "terra/rover/3/localization",
                br#"{"version":1,"mode":"geographic","originLatitude":38.8297,"originLongitude":-77.3075,"frameID":"campus"}"#.as_slice(),
            )
            .wait()
            .unwrap();
        router
            .put(
                "terra/rover/3/pose",
                br#"{"x":12,"y":4,"yaw":0,"heading":0}"#.as_slice(),
            )
            .wait()
            .unwrap();
        let localizations: serde_json::Value =
            serde_json::from_str(&client.localization_snapshot()).unwrap();
        let marks = fleet_marks(&client.snapshot(), &localizations, |_| None, origin);
        let Some(mark) = marks.first() else {
            return false;
        };
        let (latitude, longitude) = Anchor {
            latitude: aligned.latitude,
            longitude: aligned.longitude,
        }
        .geographic(12., 4.)
        .unwrap();
        (mark.latitude - latitude).abs() < 1e-4 && (mark.longitude - longitude).abs() < 1e-4
    });
    drop(client);
    drop(router);
}
