//! Headless check that a Zorvane-shaped publisher moves the map's data layer.
use argos_core::{Anchor, GoalRequest, LocalFrame};
use argos_gpui::{DashboardConfig, FleetFeed, fleet_marks};
use argos_zenoh::Client;
use std::sync::{Arc, Mutex};
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
    client.focus(Some(3)).unwrap();
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
        !fleet_marks(
            &client.snapshot(),
            &located,
            &serde_json::Value::Null,
            |_| None,
            origin,
        )
        .is_empty()
    });
    let start = fleet_marks(
        &client.snapshot(),
        &serde_json::Value::Null,
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
        let marks = fleet_marks(
            &client.snapshot(),
            &localizations,
            &serde_json::Value::Null,
            |_| None,
            origin,
        );
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

#[test]
fn operator_sends_a_local_goal_a_level_and_a_shaped_twist() {
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
    let wire = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let recorded = wire.clone();
    let _inbox = router
        .declare_subscriber("terra/rover/3/**")
        .callback(move |sample| {
            let key = sample.key_expr().to_string();
            let body = String::from_utf8_lossy(&sample.payload().to_bytes()).into_owned();
            recorded.lock().unwrap().push((key, body));
        })
        .wait()
        .unwrap();
    let level = Arc::new(Mutex::new("teleop".to_string()));
    let publish_level = level.clone();
    let camera_running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let publish_camera = camera_running.clone();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let publish_stop = stop.clone();
    let publisher = router.clone();
    let pump = std::thread::spawn(move || {
        let mut sequence = 1u64;
        while !publish_stop.load(std::sync::atomic::Ordering::Relaxed) {
            let level = publish_level.lock().unwrap().clone();
            let _ = publisher
                .put(
                    "terra/rover/fleet/state",
                    br#"{"count":1,"max_count":1,"ids":[3]}"#.as_slice(),
                )
                .wait();
            let pose =
                format!(r#"{{"x":1.0,"y":0.5,"yaw":0.1,"sequence":{sequence},"tracked":true}}"#);
            if publish_camera.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = publisher.put("terra/rover/3/pose", pose).wait();
            }
            let _ = publisher
                .put(
                    "terra/rover/3/map/occupancy",
                    br#"{"schema_version":1,"rover_id":3,"run_id":"run","sequence":1,"width":2,"height":2,"resolution":0.5,"origin_x":0,"origin_y":0,"occupancy":[-1,0,64,65]}"#.as_slice(),
                )
                .wait();
            let status = format!(
                r#"{{"requested_level":"{level}","effective_level":"{level}","assigned_level":"waypoint","active_source":"none","safety":"clear","reason":"idle","revision":1,"run_id":"run","token":"status","result":"accepted","supported_levels":["teleop","assisted_teleop","waypoint","waypoint_direct","supervised","target_search"],"paused":false,"global_route":[[0.0,0.0],[1.0,0.5]]}}"#
            );
            let _ = publisher
                .put("terra/rover/3/autonomy/status", status)
                .wait();
            let _ = publisher
                .put(
                    "terra/rover/3/hardware/status",
                    br#"{"schema_version":1,"control_mode":"streamed_relay","connected":true,"ready":true,"output_active":false,"reason":"ready","age":0.1}"#,
                )
                .wait();
            sequence += 1;
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    let dashboard = DashboardConfig::parse(
        Some(&endpoint),
        Some("terra/rover"),
        Some(""),
        Some(""),
        (Some("0"), Some("0")),
        (None, None),
    )
    .unwrap();
    let feed = FleetFeed::start(dashboard);
    // The window selects the rover; only then is its grid subscribed.
    feed.focus(Some(3));
    wait_for(|| {
        let picture = feed.picture();
        picture.link == "healthy"
            && picture.marks.first().is_some_and(|mark| {
                mark.status == "online"
                    && mark.occupancy.starts_with("2x2, 1 occupied")
                    && mark.hardware.ready
                    && mark.hardware.connected
                    && mark.requested_level.as_deref() == Some("teleop")
                    && mark.recommended_level.as_deref() == Some("waypoint")
                    && !mark.outdoor_gps
                    && mark.grid.as_ref().is_some_and(|grid| {
                        grid.cells.contains(&argos_core::CellClass::Occupied)
                            && grid.cells.contains(&argos_core::CellClass::Free)
                            && grid.cells.contains(&argos_core::CellClass::Unknown)
                    })
            })
    });
    feed.begin_drive(3).unwrap();
    feed.drive_key(3, "a", true).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    feed.drive_key(3, "a", false).unwrap();
    assert!(
        wire.lock()
            .unwrap()
            .iter()
            .filter(|(key, _)| key.ends_with("/teleop"))
            .all(|(_, body)| !body.contains("\"angular\":0.3") && !body.contains("\"linear\":0.2")),
        "turn-only input published a twist"
    );
    feed.drive_key(3, "w", true).unwrap();
    wait_for(|| {
        wire.lock().unwrap().iter().any(|(key, body)| {
            key.ends_with("/teleop")
                && body.contains("\"linear\":0.2")
                && body.contains("\"angular\":0.0")
        })
    });
    feed.drive_key(3, "w", false).unwrap();
    wait_for(|| {
        wire.lock().unwrap().iter().rev().any(|(key, body)| {
            key.ends_with("/teleop")
                && body.contains("\"linear\":0.0")
                && body.contains("\"angular\":0.0")
        })
    });
    feed.drive_key(3, "w", true).unwrap();
    std::thread::sleep(Duration::from_millis(900));
    assert!(
        wire.lock().unwrap().iter().rev().any(|(key, body)| {
            key.ends_with("/teleop")
                && body.contains("\"linear\":0.0")
                && body.contains("\"angular\":0.0")
        }),
        "deadman did not return the twist to zero"
    );
    feed.end_drive();
    wire.lock().unwrap().clear();
    feed.publish_goal(
        3,
        GoalRequest::Local {
            x: 1.25,
            y: -0.5,
            yaw: None,
        },
        None,
    );
    wait_for(|| {
        wire.lock().unwrap().iter().any(|(key, body)| {
            key.ends_with("/goal")
                && body.contains("\"frame\":\"local\"")
                && body.contains("1.25")
                && body.contains("-0.5")
        })
    });
    feed.publish_goal(
        3,
        GoalRequest::Geographic {
            latitude: 38.83,
            longitude: -77.3,
            yaw: None,
        },
        Some(5.),
    );
    wait_for(|| feed.picture().command.contains("App update required"));
    assert!(
        wire.lock()
            .unwrap()
            .iter()
            .all(|(_, body)| !body.contains("wgs84"))
    );
    feed.publish_level(3, "waypoint");
    wait_for(|| {
        wire.lock().unwrap().iter().any(|(key, body)| {
            key.ends_with("/autonomy") && body.contains("\"level\":\"waypoint\"")
        })
    });
    let local_token = wire
        .lock()
        .unwrap()
        .iter()
        .find_map(|(key, body)| {
            let v: serde_json::Value = serde_json::from_str(body).ok()?;
            if key.ends_with("/goal") && v["frame"] == "local" {
                v["token"].as_str().map(String::from)
            } else {
                None
            }
        })
        .unwrap();
    router.put("terra/rover/3/goal/status",serde_json::json!({"state":"active","goal_id":1,"token":local_token,"x":1.25,"y":-0.5,"distance":2.}).to_string()).wait().unwrap();
    *level.lock().unwrap() = "waypoint".into();
    router.put("terra/rover/3/navigation/outdoor",serde_json::json!({
        "schema_version":1,"capability":"outdoor_gps","supported":true,"acknowledged":true,
        "source":"outdoor_gps","run_id":"run","navigation_revision":4,"ready":true,
        "phase":"idle","arrival_radius_m":5.,"gps_age_s":0.1,"gps_accuracy_m":1.,
        "heading_age_s":0.1,"heading_accuracy_deg":3.,"gps":{"latitude":38.83,"longitude":-77.3}
    }).to_string()).wait().unwrap();
    wait_for(|| {
        feed.picture()
            .marks
            .first()
            .is_some_and(|m| m.outdoor_gps && m.effective_level.as_deref() == Some("waypoint"))
    });
    feed.publish_goal(
        3,
        GoalRequest::Geographic {
            latitude: 38.8301,
            longitude: -77.3,
            yaw: None,
        },
        Some(2.),
    );
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(feed.picture().command, "Goal sent");
    wait_for(|| {
        wire.lock().unwrap().iter().any(|(key, body)| {
            let value: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            key.ends_with("/goal")
                && value["frame"] == "wgs84"
                && value["run_id"] == "run"
                && value["navigation_revision"] == 4
                && value["arrival_radius_m"] == 2.
        })
    });
    *level.lock().unwrap() = "teleop".into();
    let _ = router
        .put(
            "terra/rover/3/pose/quality",
            br#"{"schema_version":1,"rover_id":3,"run_id":"run","sequence":900,"degraded":true,"reason":"tracking lost","pose_age_s":0.2,"imu_age_s":null}"#,
        )
        .wait();
    wait_for(|| {
        feed.picture().marks.first().is_some_and(|mark| {
            mark.degraded
                && mark.quality_reported
                && mark.degrade_reason == "tracking lost"
                && !mark.degrade_reason.contains("fallback")
        })
    });
    feed.publish_goal(
        3,
        GoalRequest::Local {
            x: 0.5,
            y: 0.5,
            yaw: None,
        },
        None,
    );
    wait_for(|| feed.picture().command.contains("degraded"));
    camera_running.store(false, std::sync::atomic::Ordering::Relaxed);
    wait_for(|| {
        feed.picture().marks.first().is_some_and(|mark| {
            mark.status == "pose stale" && mark.requested_level.as_deref() == Some("teleop")
        })
    });
    wire.lock().unwrap().clear();
    feed.begin_drive(3).unwrap();
    feed.drive_key(3, "w", true).unwrap();
    wait_for(|| {
        wire.lock().unwrap().iter().any(|(key, body)| {
            let value: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            key.ends_with("/teleop") && value["linear"].as_f64().is_some_and(|v| v > 0.)
        })
    });
    feed.drive_key(3, "w", false).unwrap();
    feed.end_drive();
    *level.lock().unwrap() = "assisted_teleop".into();
    wait_for(|| {
        feed.picture()
            .marks
            .first()
            .is_some_and(|mark| mark.requested_level.as_deref() == Some("assisted_teleop"))
    });
    assert_eq!(feed.begin_drive(3).unwrap_err(), "Pose is stale");
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    pump.join().unwrap();
}

#[test]
fn shutdown_disconnects_so_a_replacement_feed_can_take_over() {
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
    let dashboard = DashboardConfig::parse(
        Some(&endpoint),
        Some("terra/rover"),
        Some(""),
        Some(""),
        (Some("0"), Some("0")),
        (None, None),
    )
    .unwrap();
    let healthy = |feed: &FleetFeed| {
        router
            .put(
                "terra/rover/fleet/state",
                br#"{"count":1,"max_count":1,"ids":[3]}"#.as_slice(),
            )
            .wait()
            .unwrap();
        router
            .put(
                "terra/rover/3/pose",
                br#"{"x":0,"y":0,"yaw":0,"sequence":1,"tracked":true}"#.as_slice(),
            )
            .wait()
            .unwrap();
        feed.picture().link == "healthy"
    };
    let old = FleetFeed::start(dashboard.clone());
    wait_for(|| healthy(&old));
    old.shutdown();
    assert_eq!(old.safety_reset(3), Err("Not connected".to_string()));
    let replacement = FleetFeed::start(dashboard);
    wait_for(|| healthy(&replacement));
    // Terra refuses the reset without authority, but the replacement has a session.
    assert_ne!(
        replacement.safety_reset(3),
        Err("Not connected".to_string())
    );
    // The old feed stays disconnected after its thread has had time to exit.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(old.safety_reset(3), Err("Not connected".to_string()));
    replacement.shutdown();
}

#[test]
fn found_buckets_land_on_the_map() {
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
    let origin = LocalFrame::origin(38.83, -77.3).unwrap();
    let objects = br#"{"schema_version":1,"rover_id":3,"run_id":"run","revision":4,"objects":[
        {"id":1,"class":"bucket","color":"orange","x":10.0,"y":0.0,"latitude":null,"longitude":null,"geo_accuracy_m":null,"confidence":0.9,"sightings":4,"confirmed":true,"first_seen":1.0,"last_seen":2.0},
        {"id":2,"class":"bucket","color":"blue","x":0.0,"y":0.0,"latitude":38.8301,"longitude":-77.3002,"geo_accuracy_m":2.5,"confidence":0.8,"sightings":3,"confirmed":true,"first_seen":1.0,"last_seen":2.0}]}"#;
    let mut marks = Vec::new();
    wait_for(|| {
        router
            .put(
                "terra/rover/fleet/state",
                br#"{"count":1,"max_count":1,"ids":[3]}"#.as_slice(),
            )
            .wait()
            .unwrap();
        router
            .put(
                "terra/rover/3/pose",
                br#"{"x":0,"y":0,"yaw":0,"heading":0}"#.as_slice(),
            )
            .wait()
            .unwrap();
        router
            .put("terra/rover/3/objects", objects.as_slice())
            .wait()
            .unwrap();
        let operator: serde_json::Value =
            serde_json::from_str(&client.operator_snapshot()).unwrap();
        marks = fleet_marks(
            &client.snapshot(),
            &serde_json::Value::Null,
            &operator,
            |_| None,
            origin,
        );
        marks.first().is_some_and(|mark| mark.objects.len() == 2)
    });
    let found = &marks[0].objects;
    // No GPS on the first bucket: it goes through the map frame, 10 m from the rover.
    assert!(!found[0].from_gps);
    let metres = argos_core::geodesic_metres(
        marks[0].latitude,
        marks[0].longitude,
        found[0].latitude,
        found[0].longitude,
    );
    assert!((metres - 10.).abs() < 0.1, "{metres}");
    // The second carries its own GPS position.
    assert!(found[1].from_gps);
    assert_eq!((found[1].latitude, found[1].longitude), (38.8301, -77.3002));
}
