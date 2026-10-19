use argos_core::{
    APP_UPDATE_REQUIRED, ARRIVAL_RADIUS_DEFAULT_M, GoalRequest, OutdoorAdmission, OutdoorReport,
    admit_outdoor_goal, app_update_required, confirm_geographic_goal, decode_raw_gps,
    draft_map_click, encode_source_request, freeze_arrival_radius, geodesic_metres, hold_label,
    reported_outdoor, source_change_refusal,
};
use serde_json::json;

fn ready() -> OutdoorReport {
    OutdoorReport::decode(&json!({
        "schema_version": 1,
        "capability": "outdoor_gps",
        "supported": true,
        "acknowledged": true,
        "source": "outdoor_gps",
        "run_id": "run",
        "navigation_revision": 4,
        "ready": true,
        "hold_reason": null,
        "gps_age_s": 0.4,
        "gps_accuracy_m": 4.0,
        "heading_age_s": 0.2,
        "heading_accuracy_deg": 8.0,
        "arrival_radius_m": 5.0,
        "destination": {"latitude": 38.8297, "longitude": -77.3075},
        "remaining_m": 12.0,
        "phase": "planning",
        "gps": {"latitude": 38.8297, "longitude": -77.3075, "accuracy_m": 4.0, "age_s": 0.4},
        "token": "ack",
        "result": "accepted",
        "age": 0.2
    }))
    .unwrap()
}

fn gate<'a>(report: Option<&'a OutdoorReport>) -> OutdoorAdmission<'a> {
    OutdoorAdmission {
        report,
        latitude: 38.8299,
        longitude: -77.3075,
        arrival_radius_m: 5.,
        level: Some("waypoint"),
        run_id: "run",
        navigation_revision: 4,
    }
}

#[test]
fn missing_capability_asks_for_an_app_update_and_a_ready_report_admits() {
    assert!(app_update_required(None));
    assert_eq!(
        admit_outdoor_goal(gate(None)).unwrap_err(),
        APP_UPDATE_REQUIRED
    );
    let report = ready();
    assert!(report.acknowledged_outdoor());
    admit_outdoor_goal(gate(Some(&report))).unwrap();
    let nested = reported_outdoor(&json!({"authority": {"navigation": {"outdoor": {
        "schema_version": 1,
        "capability": "outdoor_gps",
        "supported": true,
        "acknowledged": false,
        "source": "local",
        "run_id": "run",
        "navigation_revision": "4"
    }}}}))
    .unwrap();
    assert!(!nested.acknowledged_outdoor());
    assert_eq!(nested.navigation_revision, 4);
}

#[test]
fn readiness_fails_closed_and_hold_reasons_stay_distinct() {
    let mut report = ready();
    report.gps_age_s = Some(3.1);
    assert_eq!(
        admit_outdoor_goal(gate(Some(&report))).unwrap_err(),
        "GPS stale"
    );
    report = ready();
    report.gps_accuracy_m = Some(6.);
    report.arrival_radius_m = 5.;
    let mut admission = gate(Some(&report));
    admission.arrival_radius_m = 5.;
    assert_eq!(
        admit_outdoor_goal(admission).unwrap_err(),
        "GPS uncertainty is larger than the arrival radius"
    );
    report = ready();
    report.gps_accuracy_m = Some(11.);
    assert_eq!(
        admit_outdoor_goal(gate(Some(&report))).unwrap_err(),
        "GPS uncertainty exceeds 10 m"
    );
    report = ready();
    report.heading_accuracy_deg = Some(31.);
    assert_eq!(
        admit_outdoor_goal(gate(Some(&report))).unwrap_err(),
        "Heading uncertain"
    );
    report = ready();
    report.heading_age_s = Some(2.1);
    assert_eq!(
        admit_outdoor_goal(gate(Some(&report))).unwrap_err(),
        "Heading uncertain"
    );
    report = ready();
    report.gps_latitude = Some(38.8297 + 0.01);
    assert_eq!(
        admit_outdoor_goal(gate(Some(&report))).unwrap_err(),
        "Mission distance exceeds 500 m"
    );
    report = ready();
    report.navigation_revision = 5;
    assert!(
        admit_outdoor_goal(gate(Some(&report)))
            .unwrap_err()
            .contains("revision")
    );
    report = ready();
    report.ready = true;
    report.hold_reason = Some("paused".into());
    assert_eq!(
        admit_outdoor_goal(gate(Some(&report))).unwrap_err(),
        "Paused"
    );
    for (token, label) in [
        ("gps_stale", "GPS stale"),
        ("heading_uncertain", "Heading uncertain"),
        ("tracking_lost", "Tracking lost"),
        ("obstacle_map_stale", "Obstacle map stale"),
        ("route_blocked", "Route blocked"),
        ("wheel_feedback_unavailable", "Wheel feedback unavailable"),
        ("paused", "Paused"),
    ] {
        assert_eq!(hold_label(token), Some(label));
        report.hold_reason = Some(token.into());
        assert_eq!(admit_outdoor_goal(gate(Some(&report))).unwrap_err(), label);
    }
}

#[test]
fn map_click_drafts_and_confirm_sends_the_same_wgs84_point() {
    let report = ready();
    let draft = draft_map_click(3, 38.8299, -77.3075, Some(&report), 5.).unwrap();
    assert!((draft.arrival_radius_m - 5.).abs() < 1e-9);
    assert!(draft_map_click(3, 38.8299, -77.3075, None, 5.).is_err());
    let mut local = report.clone();
    local.source = "local".into();
    local.acknowledged = true;
    assert_eq!(
        draft_map_click(3, 38.8299, -77.3075, Some(&local), 5.).unwrap_err(),
        "Outdoor GPS is not acknowledged"
    );
    let goal = confirm_geographic_goal(&draft, Some(&report), Some("waypoint"), "run").unwrap();
    match goal {
        GoalRequest::Geographic {
            latitude,
            longitude,
            yaw: None,
        } => {
            assert_eq!(latitude, draft.latitude);
            assert_eq!(longitude, draft.longitude);
        }
        other => panic!("destination changed: {other:?}"),
    }
    let bytes = goal.encode("goal-1").unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["frame"], "wgs84");
    assert_eq!(body["latitude"], 38.8299);
    assert_eq!(body["longitude"], -77.3075);
    assert!(body.get("arrival_radius").is_none());
    assert!(
        confirm_geographic_goal(&draft, Some(&report), Some("waypoint_direct"), "run").is_err()
    );
    let mut stale = report.clone();
    stale.gps_age_s = Some(9.);
    assert!(confirm_geographic_goal(&draft, Some(&stale), Some("waypoint"), "run").is_err());
    assert_eq!(
        freeze_arrival_radius(None, 5.).unwrap(),
        ARRIVAL_RADIUS_DEFAULT_M
    );
    assert!(freeze_arrival_radius(Some(1.), 5.).is_err());
    assert!(freeze_arrival_radius(Some(20.), 5.).is_ok());
}

#[test]
fn source_changes_stay_disarmed_and_raw_gps_survives_a_hold() {
    let report = ready();
    assert!(
        source_change_refusal(false, false, false, "outdoor_gps", Some(&report))
            .unwrap()
            .contains("disarmed")
    );
    assert!(
        source_change_refusal(true, true, false, "local", Some(&report))
            .unwrap()
            .contains("mission")
    );
    assert_eq!(
        source_change_refusal(true, false, false, "outdoor_gps", None),
        Some(APP_UPDATE_REQUIRED)
    );
    assert!(source_change_refusal(true, false, false, "local", None).is_none());
    let bytes = encode_source_request("outdoor_gps", "src-1", "run", 4).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["source"], "outdoor_gps");
    assert_eq!(body["navigation_revision"], 4);
    assert_eq!(body["schema_version"], 1);
    let gps = decode_raw_gps(&json!({
        "version": 1,
        "mode": "local",
        "tracking": "limited",
        "age": 1.0,
        "gpsFixAge": 4.0,
        "gpsLatitude": 38.8297,
        "gpsLongitude": -77.3075,
        "gpsAccuracy": 12.0
    }))
    .unwrap();
    assert_eq!(gps.latitude, 38.8297);
    assert!((gps.age_s.unwrap() - 5.).abs() < 1e-9);
    assert!(geodesic_metres(0., 0., 0., 0.) < 1e-6);
}
