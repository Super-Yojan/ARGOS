use super::*;
use argos_core::{Anchor, OccupancyGrid, Pose, RoverSnapshot};

fn pose(x: f64, y: f64, yaw: f64) -> Pose {
    Pose {
        rover_id: 3,
        sequence: 0,
        x,
        y,
        yaw,
    }
}
fn rover(id: u64, membership: &str, pose: Option<Pose>, pose_age: Option<f64>) -> RoverSnapshot {
    RoverSnapshot {
        id,
        membership: membership.into(),
        pose,
        pose_age,
        goal: None,
        goal_age: None,
        goal_received_at: None,
        command_phase: "none".into(),
        command_token: None,
    }
}

#[test]
fn unset_env_defaults_to_the_hosted_fleet_and_asks_for_a_password() {
    let config =
        DashboardConfig::parse(None, None, None, None, (None, None), (None, None)).unwrap();
    assert_eq!(config.endpoint, DEFAULT_ENDPOINT);
    assert_eq!(config.prefix, DEFAULT_PREFIX);
    assert!(config.credentials.is_none());
    assert_eq!(config.connect_notice.as_deref(), Some(PASSWORD_NOTICE));
    assert_eq!(config.start_latitude, DEFAULT_START_LATITUDE);
    assert_eq!(config.start_longitude, DEFAULT_START_LONGITUDE);
    assert_eq!(config.origin.latitude, DEFAULT_START_LATITUDE);
    let picture = FleetFeed::start(config).picture();
    assert_eq!(picture.link, "password");
    assert_eq!(picture.detail, PASSWORD_NOTICE);
    assert!(picture.marks.is_empty());
}

#[test]
fn password_from_the_environment_is_kept_and_local_zorvane_clears_the_user() {
    let hosted = DashboardConfig::parse(
        None,
        None,
        None,
        Some("from-the-operator"),
        (None, None),
        (None, None),
    )
    .unwrap();
    assert_eq!(
        hosted.credentials,
        Some((DEFAULT_USER.into(), "from-the-operator".into()))
    );
    assert!(hosted.connect_notice.is_none());
    let local = DashboardConfig::parse(
        Some("tcp/127.0.0.1:7447"),
        Some("terra/rover"),
        Some(""),
        Some(""),
        (None, None),
        (None, None),
    )
    .unwrap();
    assert_eq!(local.endpoint, "tcp/127.0.0.1:7447");
    assert_eq!(local.prefix, "terra/rover");
    assert!(local.credentials.is_none());
    assert!(local.connect_notice.is_none());
}

#[test]
fn dotenv_yields_to_the_environment_and_the_current_directory_overrides_the_workspace() {
    let dir = std::env::temp_dir().join(format!("argos-dotenv-{}", std::process::id()));
    let root = dir.join("root");
    let cwd = dir.join("cwd");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(
        root.join(".env"),
        "ARGOS_ZENOH_ENDPOINT=tls/from-workspace:7448\nARGOS_ZENOH_PREFIX=terra/phone\nARGOS_ZENOH_PASSWORD=from-workspace\n",
    )
    .unwrap();
    std::fs::write(
        cwd.join(".env"),
        "ARGOS_ZENOH_PREFIX=terra/rover\nARGOS_ZENOH_USER=local\n",
    )
    .unwrap();
    assert!(read_dotenv(dir.join("missing.env").as_path()).is_empty());
    let process = BTreeMap::from([("ARGOS_ZENOH_ENDPOINT".into(), "tcp/from-env:1".into())]);
    let merged = merge_dotenv(
        &process,
        &[
            read_dotenv(&root.join(".env")),
            read_dotenv(&cwd.join(".env")),
        ],
    );
    let value = |key: &str| {
        process
            .get(key)
            .or_else(|| merged.get(key))
            .map(String::as_str)
    };
    assert_eq!(value("ARGOS_ZENOH_ENDPOINT"), Some("tcp/from-env:1"));
    assert_eq!(value("ARGOS_ZENOH_PREFIX"), Some("terra/rover"));
    assert_eq!(value("ARGOS_ZENOH_USER"), Some("local"));
    assert_eq!(value("ARGOS_ZENOH_PASSWORD"), Some("from-workspace"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn maps_key_loads_from_dotenv_and_the_environment_overrides_the_file() {
    let dir = std::env::temp_dir().join(format!("argos-maps-key-{}", std::process::id()));
    let root = dir.join("root");
    let cwd = dir.join("cwd");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(
        root.join(".env"),
        "GOOGLE_MAPS_API_KEY=from-workspace-file\n# GOOGLE_MAPS_API_KEY=commented-placeholder\n",
    )
    .unwrap();
    std::fs::write(cwd.join(".env"), "GOOGLE_MAPS_API_KEY=from-cwd-file\n").unwrap();
    let absent = BTreeMap::new();
    let from_files = merge_dotenv(
        &absent,
        &[
            read_dotenv(&root.join(".env")),
            read_dotenv(&cwd.join(".env")),
        ],
    );
    assert_eq!(
        maps_api_key(&absent, &from_files).as_deref(),
        Some("from-cwd-file")
    );
    let process = BTreeMap::from([(MAPS_API_KEY_ENV.into(), "from-environment".into())]);
    let overridden = merge_dotenv(
        &process,
        &[
            read_dotenv(&root.join(".env")),
            read_dotenv(&cwd.join(".env")),
        ],
    );
    assert_eq!(
        maps_api_key(&process, &overridden).as_deref(),
        Some("from-environment")
    );
    assert!(!overridden.contains_key(MAPS_API_KEY_ENV));
    let blank = BTreeMap::from([(MAPS_API_KEY_ENV.into(), "   ".into())]);
    assert!(maps_api_key(&blank, &from_files).is_none());
    assert_eq!(MAPS_KEY_NOTICE, "Set GOOGLE_MAPS_API_KEY in .env");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn config_rejects_partial_pairs_and_bad_numbers() {
    assert!(
        DashboardConfig::parse(
            Some("udp/127.0.0.1:1"),
            None,
            None,
            None,
            (None, None),
            (None, None)
        )
        .is_err()
    );
    assert!(
        DashboardConfig::parse(
            Some("tcp/127.0.0.1:7447"),
            None,
            Some("terra"),
            None,
            (None, None),
            (None, None)
        )
        .is_err()
    );
    assert!(
        DashboardConfig::parse(None, None, None, None, (Some("nope"), None), (None, None)).is_err()
    );
    assert!(
        DashboardConfig::parse(None, None, None, None, (None, None), (Some("10"), None)).is_err()
    );
    let config = DashboardConfig::parse(
        None,
        Some("terra/phone"),
        None,
        None,
        (Some("10"), Some("20")),
        (Some("1"), Some("2")),
    )
    .unwrap();
    assert_eq!(config.prefix, "terra/phone");
    assert_eq!(config.start_latitude, 10.);
    assert_eq!(config.origin.latitude, 1.);
    assert_eq!(config.origin.longitude, 2.);
}

#[test]
fn marks_follow_localization_and_summarize_occupancy() {
    let origin = LocalFrame::origin(0., 0.).unwrap();
    let mut pose = pose(12., 4., std::f64::consts::FRAC_PI_2);
    pose.rover_id = 3;
    let snapshot = Snapshot {
        link: "healthy".into(),
        fleet_age: Some(0.1),
        rovers: vec![
            rover(3, "online", Some(pose), Some(0.2)),
            rover(4, "online", None, None),
        ],
        notice: None,
    };
    let localizations = serde_json::json!({
        "3": {
            "version": 1,
            "mode": "geographic",
            "originLatitude": 38.8297,
            "originLongitude": -77.3075
        }
    });
    let grid = OccupancyGrid {
        schema_version: 1,
        rover_id: 3,
        run_id: "run".into(),
        sequence: 1,
        width: 2,
        height: 1,
        resolution: 0.5,
        origin_x: 0.,
        origin_y: 0.,
        occupancy: vec![0, 65],
    };
    let marks = fleet_marks(
        &snapshot,
        &localizations,
        &Value::Null,
        |id| (id == 3).then(|| (grid.clone(), 0.4)),
        origin,
    );
    assert_eq!(marks.len(), 1);
    let (expected_lat, expected_lon) = Anchor {
        latitude: 38.8297,
        longitude: -77.3075,
    }
    .geographic(12., 4.)
    .unwrap();
    assert!((marks[0].latitude - expected_lat).abs() < 1e-6);
    assert!((marks[0].longitude - expected_lon).abs() < 1e-6);
    assert!((marks[0].heading - 270.).abs() < 1e-6);
    assert_eq!(marks[0].status, "online");
    assert_eq!(marks[0].occupancy, "2x1, 1 occupied, 0.4s");
}

#[test]
fn local_grid_preserves_phone_resolution_and_narrow_free_gap() {
    let snapshot = Snapshot {
        link: "healthy".into(),
        fleet_age: Some(0.1),
        notice: None,
        rovers: vec![rover(3, "online", Some(pose(0., 0., 0.)), Some(0.1))],
    };
    let mut cells = vec![-1; 200 * 200];
    cells[100 * 200 + 99] = 100;
    cells[100 * 200 + 100] = 0;
    cells[100 * 200 + 101] = 100;
    let grid = OccupancyGrid {
        schema_version: 1,
        rover_id: 3,
        run_id: "run".into(),
        sequence: 1,
        width: 200,
        height: 200,
        resolution: 0.1,
        origin_x: -10.,
        origin_y: -10.,
        occupancy: cells,
    };
    let marks = fleet_marks(
        &snapshot,
        &Value::Null,
        &Value::Null,
        |_| Some((grid.clone(), 0.1)),
        LocalFrame::origin(0., 0.).unwrap(),
    );
    let map = marks[0].grid.as_ref().unwrap();
    assert_eq!((map.cols, map.rows), (200, 200));
    assert_eq!(map.cells[100 * 200 + 100], argos_core::CellClass::Free);
    assert_eq!(map.cells[100 * 200 + 99], argos_core::CellClass::Occupied);
    assert_eq!(map.cells[100 * 200 + 101], argos_core::CellClass::Occupied);
}

#[test]
fn hardware_status_is_copied_onto_the_rover() {
    let origin = LocalFrame::origin(0., 0.).unwrap();
    let snapshot = Snapshot {
        link: "healthy".into(),
        fleet_age: Some(0.1),
        rovers: vec![rover(3, "online", Some(pose(0., 0., 0.)), Some(0.1))],
        notice: None,
    };
    let operator = serde_json::json!({
        "3": {
            "authority": {
                "requested_level": "waypoint",
                "run_id": "run",
                "revision": 4,
                "safety": "clear"
            },
            "hardware": {
                "schema_version": 1,
                "control_mode": "streamed_relay",
                "connected": true,
                "ready": false,
                "output_active": false,
                "fault": null,
                "reason": "Motor relay is not ready",
                "age": 0.2
            }
        }
    });
    let marks = fleet_marks(&snapshot, &Value::Null, &operator, |_| None, origin);
    assert!(marks[0].hardware.present);
    assert!(marks[0].hardware.connected);
    assert!(!marks[0].hardware.ready);
    assert!(!marks[0].hardware.output_active);
    assert!(marks[0].hardware.fault.is_none());
    assert_eq!(marks[0].hardware.reason, "Motor relay is not ready");
    assert_eq!(marks[0].authority_revision, 4);
    assert_eq!(marks[0].safety, "clear");
    assert_eq!(marks[0].run_id, "run");
    assert_eq!(
        drive_block(&marks[0].hardware),
        Some("Motor relay is not ready")
    );

    let mut ready = operator;
    ready["3"]["hardware"]["ready"] = serde_json::json!(true);
    ready["3"]["hardware"]["reason"] = serde_json::json!("ready");
    let ready_marks = fleet_marks(&snapshot, &Value::Null, &ready, |_| None, origin);
    assert!(ready_marks[0].hardware.ready);
    assert!(!ready_marks[0].hardware.output_active);
    assert!(ready_marks[0].hardware.ready_and_neutral());
    assert_eq!(drive_block(&ready_marks[0].hardware), None);
}

#[test]
fn local_phone_gps_places_rover_without_inventing_geographic_alignment() {
    let origin = LocalFrame::origin(DEFAULT_START_LATITUDE, DEFAULT_START_LONGITUDE).unwrap();
    let snapshot = Snapshot {
        link: "healthy".into(),
        fleet_age: Some(0.1),
        notice: None,
        rovers: vec![rover(3, "online", Some(pose(2., 4., 0.)), Some(0.1))],
    };
    let report = serde_json::json!({"3":{"version":1,"mode":"local",
        "gpsLatitude":38.98205,"gpsLongitude":-77.52052,"gpsAccuracy":4.6,"gpsFixAge":0.2}});
    let marks = fleet_marks(&snapshot, &report, &Value::Null, |_| None, origin);
    assert_eq!(
        (marks[0].latitude, marks[0].longitude),
        (38.98205, -77.52052)
    );
    assert_eq!((marks[0].local_x, marks[0].local_y), (2., 4.));
    assert!(marks[0].map_frame.is_none());
}

#[test]
fn gps_rover_pins_its_local_map_under_the_marker_by_fused_heading() {
    let origin = LocalFrame::origin(DEFAULT_START_LATITUDE, DEFAULT_START_LONGITUDE).unwrap();
    // Local yaw 0 (+x) is facing true east.
    let snapshot = Snapshot {
        link: "healthy".into(),
        fleet_age: Some(0.1),
        notice: None,
        rovers: vec![rover(3, "online", Some(pose(2., 4., 0.)), Some(0.1))],
    };
    let report = serde_json::json!({"3":{"version":1,"mode":"gps",
        "gpsLatitude":38.98205,"gpsLongitude":-77.52052,"gpsAccuracy":4.6,"gpsFixAge":0.2}});
    let operator = serde_json::json!({"3":{"outdoor":{"schema_version":1,
        "capability":"outdoor_gps","supported":true,"source":"outdoor_gps","run_id":"run",
        "navigation_revision":1,"heading_deg":90.0,"heading_accuracy_deg":6.0}}});
    let marks = fleet_marks(&snapshot, &report, &operator, |_| None, origin);
    assert!(marks[0].map_frame.is_none());
    let frame = marks[0].overlay_frame.unwrap();
    let (latitude, longitude) = frame.geographic(2., 4.).unwrap();
    assert!((latitude - 38.98205).abs() < 1e-9 && (longitude + 77.52052).abs() < 1e-9);
    // One metre ahead of the rover is one metre east of the fix.
    let (latitude, longitude) = frame.geographic(3., 4.).unwrap();
    assert!((latitude - 38.98205).abs() < 1e-7);
    assert!(longitude > -77.52052);
    let east = argos_core::geodesic_metres(38.98205, -77.52052, latitude, longitude);
    assert!((east - 1.).abs() < 0.01, "{east}");
    assert!((marks[0].heading - 90.).abs() < 1e-9);

    // No heading yet: the grid turns with the marker's yaw instead.
    let marks = fleet_marks(&snapshot, &report, &Value::Null, |_| None, origin);
    let frame = marks[0].overlay_frame.unwrap();
    assert_eq!(frame.rotation, 0.);
    assert_eq!(marks[0].heading, map_heading_degrees(0.));
}

#[test]
fn camera_reframes_when_existing_rover_gets_its_geographic_location() {
    let origin = LocalFrame::origin(0., 0.).unwrap();
    let snapshot = Snapshot {
        link: "healthy".into(),
        fleet_age: Some(0.1),
        notice: None,
        rovers: vec![rover(3, "online", Some(pose(0., 0., 0.)), Some(0.1))],
    };
    let mut marks = fleet_marks(&snapshot, &Value::Null, &Value::Null, |_| None, origin);
    let ids = BTreeSet::from([3]);
    assert!(reframing_target(&ids, Some((0., 0.)), &marks).is_none());
    marks[0].latitude = 38.98205;
    marks[0].longitude = -77.52052;
    let (_, lat, lon) = reframing_target(&ids, Some((0., 0.)), &marks).unwrap();
    assert_eq!((lat, lon), (38.98205, -77.52052));
    marks[0].latitude += 0.00001; // GPS jitter must not restart camera flights.
    assert!(reframing_target(&ids, Some((lat, lon)), &marks).is_none());
    marks[0].latitude += 0.001;
    assert!(reframing_target(&ids, Some((lat, lon)), &marks).is_some());
    marks[0].status = "pose stale".into();
    assert!(reframing_target(&ids, Some((lat, lon)), &marks).is_none());
}

#[test]
fn camera_fits_when_the_rover_set_changes() {
    let origin = LocalFrame::origin(DEFAULT_START_LATITUDE, DEFAULT_START_LONGITUDE).unwrap();
    let mut first = pose(0., 0., 0.);
    first.rover_id = 1;
    let snapshot = Snapshot {
        link: "healthy".into(),
        fleet_age: Some(0.),
        rovers: vec![rover(1, "online", Some(first.clone()), Some(0.1))],
        notice: None,
    };
    let marks = fleet_marks(&snapshot, &Value::Null, &Value::Null, |_| None, origin);
    let previous = BTreeSet::new();
    let (ids, latitude, longitude) = framing_target(&previous, &marks).unwrap();
    assert_eq!(ids, BTreeSet::from([1]));
    assert!((latitude - marks[0].latitude).abs() < 1e-9);
    assert!((longitude - marks[0].longitude).abs() < 1e-9);
    assert!(framing_target(&ids, &marks).is_none());

    let mut moved = first.clone();
    moved.x = 30.;
    let moved_snapshot = Snapshot {
        rovers: vec![rover(1, "online", Some(moved), Some(3.))],
        ..snapshot.clone()
    };
    let moved_marks = fleet_marks(
        &moved_snapshot,
        &Value::Null,
        &Value::Null,
        |_| None,
        origin,
    );
    assert_eq!(moved_marks[0].status, "pose stale");
    assert!(moved_marks[0].latitude > marks[0].latitude);
    assert!(framing_target(&ids, &moved_marks).is_none());

    let mut second = pose(10., 0., 0.);
    second.rover_id = 2;
    let both = Snapshot {
        rovers: vec![
            rover(1, "absent", Some(first), Some(1.)),
            rover(2, "online", Some(second), Some(0.1)),
        ],
        ..snapshot
    };
    let both_marks = fleet_marks(&both, &Value::Null, &Value::Null, |_| None, origin);
    assert_eq!(
        both_marks.len(),
        1,
        "explicit retirement removes the old marker, keeping its presence row"
    );
    let (ids, latitude, _) = framing_target(&ids, &both_marks).unwrap();
    assert_eq!(ids, BTreeSet::from([2]));
    assert!((latitude - both_marks[0].latitude).abs() < 1e-9);
    assert!(framing_target(&BTreeSet::new(), &[]).is_none());
}

#[test]
fn membership_without_geographic_position_remains_visible() {
    let snapshot = Snapshot {
        link: "healthy".into(),
        fleet_age: Some(0.1),
        notice: None,
        rovers: vec![
            rover(0, "online", Some(pose(0., 0., 0.)), Some(0.1)),
            rover(1, "online", None, None),
        ],
    };
    let localizations = serde_json::json!({"0":{"version":1,"mode":"local","frameID":"a"}});
    let marks = fleet_marks(
        &snapshot,
        &localizations,
        &Value::Null,
        |_| None,
        LocalFrame::origin(38., -77.).unwrap(),
    );
    let rows = fleet_presence(&snapshot, &marks);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].position_reason(), "Position unavailable");
    assert_eq!(describe(&snapshot, &marks), "2 rovers");
    assert!(marks.is_empty());
}

#[test]
fn retained_fix_ages_and_cannot_cross_frame_or_retirement() {
    let mut history = LocalizationHistory::default();
    let initial = serde_json::json!({"0":{"version":1,"mode":"local","frameID":"a","gpsLatitude":38.,"gpsLongitude":-77.,"gpsFixAge":8.2,"age":0.}});
    history.update(&initial, 10.);
    let missing = serde_json::json!({"0":{"version":1,"mode":"local","frameID":"a","reason":"permission denied","age":0.}});
    let retained = history.update(&missing, 20.);
    assert!((decode_raw_gps(&retained["0"]).unwrap().age_s.unwrap() - 18.2).abs() < 1e-6);
    let offline = history.update(&Value::Null, 130.);
    assert!((decode_raw_gps(&offline["0"]).unwrap().age_s.unwrap() - 128.2).abs() < 1e-6);
    let fresh_frame = serde_json::json!({"0":{"version":1,"mode":"local","frameID":"b","age":0.}});
    assert!(decode_raw_gps(&history.update(&fresh_frame, 131.)["0"]).is_none());
    history.retain(&BTreeSet::new());
    assert_eq!(history.update(&Value::Null, 132.), serde_json::json!({}));
}

#[test]
fn source_run_mismatch_cannot_reuse_a_geographic_placement() {
    let report = serde_json::json!({"version":1,"mode":"geographic","frameID":"a","run_id":"old","originLatitude":38.,"originLongitude":-77.,"gpsLatitude":38.,"gpsLongitude":-77.});
    let value = scope_localization(report, Some("new"));
    assert!(decode_raw_gps(&value).is_none());
    assert!(LocalFrame::from_localization(&value).is_none());
    assert_eq!(value["run_id"], "new");
    assert!(value["reason"].as_str().unwrap().contains("run"));
}

#[test]
fn disconnected_picture_keeps_identity_and_ages_without_renewing_observations() {
    let snapshot = Snapshot {
        link: "healthy".into(),
        fleet_age: Some(0.1),
        notice: None,
        rovers: vec![
            rover(0, "online", Some(pose(0., 0., 0.)), Some(0.1)),
            rover(0, "online", None, None),
        ],
    };
    let rows = fleet_presence(&snapshot, &[]);
    assert_eq!(rows.len(), 1);
    let picture = Mutex::new(FleetPicture {
        presence: rows,
        ..Default::default()
    });
    mark_disconnected(&picture);
    let current = picture.lock().unwrap();
    assert_eq!(current.presence[0].id(), 0);
    assert_eq!(current.presence[0].membership(), "offline");
    assert_eq!(current.presence[0].observation_age(), Some(0.1));
}

#[test]
fn retained_marker_survives_replacement_membership_without_pose() {
    let mut previous = FleetPicture::default();
    let initial = Snapshot {
        link: "healthy".into(),
        fleet_age: Some(0.),
        notice: None,
        rovers: vec![rover(0, "online", Some(pose(0., 0., 0.)), Some(0.))],
    };
    let report = serde_json::json!({"0":{"version":1,"mode":"local","frameID":"a","gpsLatitude":38.,"gpsLongitude":-77.,"gpsFixAge":8.2}});
    previous.marks = fleet_marks(
        &initial,
        &report,
        &Value::Null,
        |_| None,
        LocalFrame::origin(0., 0.).unwrap(),
    );
    let next = Snapshot {
        rovers: vec![rover(0, "online", None, None)],
        ..initial
    };
    let mut marks = fleet_marks(
        &next,
        &report,
        &Value::Null,
        |_| None,
        LocalFrame::origin(0., 0.).unwrap(),
    );
    retain_geographic_markers(&next, &report, &previous, &mut marks);
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].raw_gps.unwrap().age_s, Some(8.2));
    assert!(!marks[0].control_pose_ready());
    assert!(marks[0].map_frame.is_none());
}
#[test]
fn retirement_rejects_cached_report_on_reused_id_and_display_override_applies() {
    let mut history = LocalizationHistory::default();
    let report = serde_json::json!({"0":{"version":1,"mode":"local","frameID":"a","age":0.,"gpsLatitude":38.,"gpsLongitude":-77.,"gpsFixAge":0.}});
    history.observe_membership(&[(0, "online")], 1.);
    history.update(&report, 1.);
    history.observe_membership(&[(0, "absent")], 2.);
    let old = serde_json::json!({"0":{"version":1,"mode":"local","frameID":"a","age":2.,"gpsLatitude":38.,"gpsLongitude":-77.,"gpsFixAge":0.}});
    assert!(decode_raw_gps(&history.update(&old, 3.)["0"]).is_none());
    history.retain(&BTreeSet::new()); // polling prunes absent observation payloads
    history.observe_membership(&[(0, "online")], 4.);
    assert_eq!(
        history.update(&Value::Null, 4.)["0"]["mode"],
        "local",
        "missing localization after retirement must not invoke the default origin"
    );
    assert!(decode_raw_gps(&history.update(&old, 4.)["0"]).is_none());
    let snapshot = Snapshot {
        link: "stale".into(),
        fleet_age: Some(100.),
        notice: None,
        rovers: vec![rover(0, "online", Some(pose(0., 0., 0.)), Some(0.))],
    };
    let report = serde_json::json!({"0":{"version":1,"mode":"local","gpsLatitude":38.,"gpsLongitude":-77.,"gpsFixAge":8.2,"gpsFresh":true,"connectivity_policy":{"gps_display_freshness_seconds":10.,"fleet_display_stale_seconds":120.}}});
    let marks = fleet_marks(
        &snapshot,
        &report,
        &Value::Null,
        |_| None,
        LocalFrame::origin(0., 0.).unwrap(),
    );
    assert!(position_label(&marks[0]).starts_with("GPS observation"));
    assert_eq!(display_link(&snapshot, &report, &Value::Null), "healthy");
    let mut revoked = report.clone();
    revoked["0"]["gpsFresh"] = Value::Bool(false);
    let marks = fleet_marks(
        &snapshot,
        &revoked,
        &Value::Null,
        |_| None,
        LocalFrame::origin(0., 0.).unwrap(),
    );
    assert!(position_label(&marks[0]).starts_with("Last known GPS"));
}
