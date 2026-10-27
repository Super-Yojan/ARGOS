use argos_core::*;

#[test]
fn level_and_teleop_payloads_round_trip() {
    assert!(encode_level_request("supervised", "mode-0").is_err());
    let level = encode_level_request("target_search", "mode-1").unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&level).unwrap(),
        serde_json::json!({"level": "target_search", "token": "mode-1"})
    );
    assert!(encode_level_request("explore", "mode-1").is_err());
    assert!(encode_level_request("teleop", "").is_err());
    let status = br#"{"requested_level":"waypoint","effective_level":"waypoint","assigned_level":"supervised","supported_levels":["teleop","waypoint"],"global_route":[[0.0,0.0],[1.5,0.25]],"revision":4,"token":"mode-1","result":"accepted"}"#;
    let report = decode_autonomy_status(status).unwrap();
    assert_eq!(report.requested, "waypoint");
    assert_eq!(report.effective.as_deref(), Some("waypoint"));
    assert_eq!(report.assigned.as_deref(), Some("supervised"));
    assert_eq!(
        report.route,
        vec![
            RouteSample {
                x: 0.,
                y: 0.,
                travel: Travel::Unspecified,
            },
            RouteSample {
                x: 1.5,
                y: 0.25,
                travel: Travel::Unspecified,
            },
        ]
    );
    assert!(report.commanded_linear.is_none());
    assert!(report.hold.is_none());
    let twist = encode_teleop(0.2, -0.1, "session-1", 3, Some("run"), Some(4)).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&twist).unwrap();
    assert_eq!(value["linear"], 0.2);
    assert_eq!(value["angular"], -0.1);
    assert_eq!(value["sequence"], 3);
    assert!(encode_teleop(0.6, 0., "session-1", 1, None, None).is_err());
}

#[test]
fn yaw_rate_is_clamped_and_slews_without_integrating_a_heading() {
    let mut guard = TeleopGuard::new(TeleopLimits {
        allow_spin: true,
        ..TeleopLimits::default()
    });
    let (_, first) = guard.step(1., 1., 0.05);
    let step = DEFAULT_DRIVE_YAW_ACCEL * 0.05;
    assert!((first - step).abs() < 1e-9);
    let (_, second) = guard.step(1., 1., 0.05);
    assert!((second - 2. * step).abs() < 1e-9);
    assert!(second < DEFAULT_DRIVE_YAW_RATE);
    let mut capped = second;
    for _ in 0..40 {
        capped = guard.step(1., 1., 0.05).1;
    }
    assert!((capped - DEFAULT_DRIVE_YAW_RATE).abs() < 1e-9);
    let (_, easing) = guard.step(1., 0., 0.05);
    assert!((capped - easing - DEFAULT_DRIVE_YAW_DECEL * 0.05).abs() < 1e-9);
    assert!(easing > 0.);
    let (_, jumped) = guard.step(1., 1., 10.);
    assert!((jumped - easing).abs() <= DEFAULT_DRIVE_YAW_ACCEL * 0.5 + 1e-9);
}

#[test]
fn in_place_spin_is_off_unless_requested_and_release_returns_to_zero() {
    let mut guard = TeleopGuard::new(TeleopLimits::default());
    assert!(!guard.limits.allow_spin);
    let (linear, angular) = guard.step(0., 1., 0.05);
    assert_eq!((linear, angular), (0., 0.));
    let (linear, angular) = guard.step(1., 1., 0.05);
    assert!((linear - DEFAULT_DRIVE_LINEAR).abs() < 1e-9);
    assert!(angular > 0.);
    guard.limits.allow_spin = true;
    let mut angular = 0.;
    for _ in 0..30 {
        angular = guard.step(0., 1., 0.05).1;
    }
    assert!(angular > 0.2);
    for _ in 0..40 {
        angular = guard.step(0., 0., 0.05).1;
    }
    assert_eq!(angular, 0.);
}

#[test]
fn deadman_expires_and_a_degraded_pose_blocks_a_new_goal() {
    let mut lease = DriveLease::default();
    assert!(!lease.alive(0.));
    lease.hold(1.);
    assert!(lease.alive(1.2));
    assert!(!lease.alive(1. + DRIVE_DEADMAN_SECONDS));
    lease.release();
    assert!(!lease.alive(1.1));

    let limits = DegradeLimits::default();
    let mut watch = PoseWatch::default();
    let (degraded, _) = watch.observe(0., 1, 0., None, limits);
    assert!(!degraded);
    assert!(goal_blocked(false).is_none());
    let (degraded, reason) = watch.observe(0.2, 2, 0.2, None, limits);
    assert!(degraded, "{reason}");
    assert_eq!(reason, "yaw rate is high");
    let (degraded, reason) = watch.observe(0.2, 3, 0.4, None, limits);
    assert!(degraded);
    assert_eq!(reason, "settling after a turn");
    let (degraded, _) = watch.observe(0.2, 4, 0.4 + DEFAULT_DEGRADED_SETTLE, None, limits);
    assert!(!degraded);
    let (degraded, reason) = watch.observe(0.2, 5, 3., Some(false), limits);
    assert!(degraded);
    assert_eq!(reason, "VIO tracking lost");
    assert_eq!(
        goal_blocked(true),
        Some("pose is degraded; not sending a goal")
    );
    let fallback = fallback_degrade_reason(reason);
    assert!(fallback.starts_with("yaw-rate fallback"));
    let value = serde_json::json!({"x": 1, "y": 2, "yaw": 0, "tracked": false});
    assert_eq!(pose_tracking_flag(&value), Some(false));
    assert_eq!(
        pose_tracking_flag(&serde_json::json!({"quality": 0.2})),
        Some(false)
    );
    assert_eq!(
        pose_tracking_flag(&serde_json::json!({"x": 0, "yaw": 0})),
        None
    );
    assert!(!outdoor_gps_advertised(&serde_json::json!({})));
    assert!(outdoor_gps_advertised(
        &serde_json::json!({"authority": {"navigation": {"source": "outdoor_gps"}}})
    ));
    assert_eq!(
        geographic_goal_refusal(false),
        Some("Outdoor GPS goals are unsupported on this rover")
    );
    assert!(geographic_goal_refusal(true).is_none());
}

#[test]
fn reverse_uses_the_same_radius_cap_and_quality_replaces_the_heuristic() {
    let mut guard = TeleopGuard::new(TeleopLimits::default());
    let mut angular = 0.;
    let mut linear = 0.;
    for _ in 0..80 {
        let sample = guard.step(-1., 1., 0.05);
        linear = sample.0;
        angular = sample.1;
    }
    let cap = DEFAULT_DRIVE_LINEAR / DEFAULT_MIN_TURN_RADIUS;
    assert!(linear < 0.);
    assert!(angular > 0.);
    assert!(angular <= cap + 1e-9);

    let quality = decode_pose_quality(
        br#"{"schema_version":1,"rover_id":3,"run_id":"run","sequence":4,"degraded":true,"reason":"tracking lost","pose_age_s":0.2,"imu_age_s":null}"#,
    )
    .unwrap();
    assert!(quality.degraded);
    assert!(quality.imu_age_s.is_none());
    assert!((quality.pose_age_s.unwrap() - 0.2).abs() < 1e-9);
    let status = serde_json::json!({
        "requested_level": "waypoint",
        "effective_level": "waypoint",
        "reason": "heading_requires_spin_or_replan",
        "commanded_linear": -0.15,
        "command_origin": "phone_bluetooth",
        "global_route": [[0.0, 0.0], [1.0, 0.0, -0.2], {"x": 1.0, "y": 1.0, "velocity": 0.2}],
        "pose_quality": {
            "schema_version": 1,
            "rover_id": 3,
            "run_id": "run",
            "sequence": 4,
            "degraded": false,
            "reason": "",
            "pose_age_s": null,
            "imu_age_s": null
        }
    });
    let report = decode_autonomy_value(&status).unwrap();
    assert_eq!(report.hold.as_deref(), Some(HEADING_HOLD));
    assert_eq!(report.commanded_linear, Some(-0.15));
    assert_eq!(report.origin, CommandOrigin::PhoneBluetooth);
    assert_eq!(report.route[1].travel, Travel::Reverse);
    assert_eq!(report.route[2].travel, Travel::Forward);
    assert_eq!(report.route[0].travel, Travel::Unspecified);
    assert!(report.pose_quality.is_some_and(|quality| !quality.degraded));
    assert!(pose_warning_text(report.origin, true, "tracking lost").is_none());
}

/// Terra's default `navigation` status, as `terra_runtime::navigation::State::status` publishes it.
fn terra_navigation_status() -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "revision": 3,
        "parameters": {
            "turning": {
                "max_yaw_rate": 0.5,
                "angular_acceleration": 0.5,
                "angular_deceleration": 0.8,
                "min_turn_radius": 0.5,
                "allow_in_place": false
            },
            "max_reverse": 0.15,
            "reverse_penalty": 0.05,
            "length": 0.85,
            "width": 0.85,
            "clearance": 0.04896,
            "max_linear": 2.0,
            "max_angular": 2.0,
            "horizon": 2.0
        },
        "footprint_radius": 0.65,
        "token": null,
        "result": null
    })
}

#[test]
fn navigation_settings_change_only_turning_and_body_size() {
    let status = terra_navigation_status();
    assert_eq!(navigation_allows_in_place(&status), Some(false));
    let bytes = encode_navigation_settings(
        &status,
        NavigationEdit::crab_trap_bot(true),
        "nav-1",
        "run-7",
        12,
    )
    .unwrap();
    let request: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut expected = status["parameters"].clone();
    expected["turning"]["allow_in_place"] = true.into();
    expected["length"] = CRAB_TRAP_BOT_LENGTH.into();
    expected["width"] = CRAB_TRAP_BOT_WIDTH.into();
    assert_eq!(
        request,
        serde_json::json!({
            "token": "nav-1",
            "run_id": "run-7",
            "authority_revision": 12,
            "settings_revision": 3,
            "parameters": expected,
        })
    );
}

#[test]
fn navigation_settings_need_reported_status_and_valid_inputs() {
    let status = terra_navigation_status();
    let edit = NavigationEdit::crab_trap_bot(true);
    for missing in [
        serde_json::Value::Null,
        serde_json::json!({"schema_version": 2, "revision": 1, "parameters": status["parameters"]}),
        serde_json::json!({"schema_version": 1, "parameters": status["parameters"]}),
        serde_json::json!({"schema_version": 1, "revision": 1, "parameters": {"length": 0.85}}),
    ] {
        assert!(encode_navigation_settings(&missing, edit, "nav-1", "run-7", 1).is_err());
    }
    for (length, width) in [(0.05, 0.6), (0.6, 3.5), (f64::NAN, 0.6)] {
        let edit = NavigationEdit {
            allow_in_place: true,
            length,
            width,
        };
        assert!(encode_navigation_settings(&status, edit, "nav-1", "run-7", 1).is_err());
    }
    assert!(encode_navigation_settings(&status, edit, "bad token", "run-7", 1).is_err());
    assert!(encode_navigation_settings(&status, edit, "nav-1", "", 1).is_err());
}

#[test]
fn autonomy_status_carries_terra_sensor_hold() {
    let status = br#"{"requested_level":"waypoint","effective_level":null,"reason":"sensor_unhealthy","sensor_hold":{"cause":"stale_vio","pose_age_s":0.41,"imu_age_s":0.01,"age_s":2.0,"count":3}}"#;
    let hold = decode_autonomy_status(status).unwrap().sensor_hold.unwrap();
    assert_eq!(hold.cause, "stale_vio");
    assert_eq!(hold.count, 3);
    assert_eq!(hold.label(), "stale_vio · pose 0.41 s old");
    let older = br#"{"requested_level":"waypoint","reason":"sensor_unhealthy"}"#;
    assert!(decode_autonomy_status(older).unwrap().sensor_hold.is_none());
}

#[test]
fn study_condition_locks_levels_like_terra() {
    use argos_core::{encode_assignment_request, level_permitted};
    assert!(level_permitted(None, "supervised"));
    assert!(level_permitted(Some("waypoint"), "waypoint"));
    assert!(level_permitted(Some("waypoint"), "teleop"));
    assert!(!level_permitted(Some("waypoint"), "waypoint_direct"));
    assert!(!level_permitted(Some("waypoint"), "supervised"));
    assert!(level_permitted(Some("explore"), "target_search"));
    assert!(!level_permitted(Some("waypoint"), "target_search"));
    let set: serde_json::Value =
        serde_json::from_slice(&encode_assignment_request(Some("explore"), "s-1").unwrap())
            .unwrap();
    assert_eq!(
        set,
        serde_json::json!({"assigned_level":"explore","token":"s-1"})
    );
    let clear: serde_json::Value =
        serde_json::from_slice(&encode_assignment_request(None, "s-2").unwrap()).unwrap();
    assert_eq!(
        clear,
        serde_json::json!({"assigned_level":null,"token":"s-2"})
    );
    assert!(encode_assignment_request(Some("supervised"), "s-3").is_err());
    assert!(encode_assignment_request(Some("waypoint"), "bad token").is_err());
}
