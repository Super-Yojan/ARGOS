use argos_core::{
    APP_UPDATE_REQUIRED, ARRIVAL_RADIUS_DEFAULT_M, OutdoorAdmission, OutdoorReport,
    admit_outdoor_goal, app_update_required, decode_raw_gps, encode_source_request,
    freeze_arrival_radius, geodesic_metres, hold_label, reported_outdoor, source_change_refusal,
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
    report.heading_accuracy_deg = Some(35.1);
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
        ("hardware_unavailable", "Hardware unavailable"),
        ("paused", "Paused"),
    ] {
        assert_eq!(hold_label(token), Some(label));
        report.hold_reason = Some(token.into());
        assert_eq!(admit_outdoor_goal(gate(Some(&report))).unwrap_err(), label);
    }
}

#[test]
fn arrival_radius_stays_in_the_reviewed_range() {
    assert_eq!(
        freeze_arrival_radius(None, ARRIVAL_RADIUS_DEFAULT_M).unwrap(),
        ARRIVAL_RADIUS_DEFAULT_M
    );
    assert!(freeze_arrival_radius(Some(1.), 5.).is_err());
    assert!(freeze_arrival_radius(Some(20.), 5.).is_ok());
    let mut local = ready();
    local.source = "local".into();
    assert_eq!(
        admit_outdoor_goal(gate(Some(&local))).unwrap_err(),
        "Outdoor GPS is not acknowledged"
    );
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
#[test]
fn resume_is_explicit_and_requires_current_paused_mission() {
    let mut report = ready();
    assert!(argos_core::outdoor_resume_refusal(&report, "run", 4).is_some());
    report.phase = "paused".into();
    report.ready = false;
    report.hold_reason = Some("gps_stale".into());
    assert!(argos_core::outdoor_resume_refusal(&report, "run", 4).is_none());
    report.gps_age_s = Some(4.);
    assert!(argos_core::outdoor_resume_refusal(&report, "run", 4).is_some());
}

#[test]
fn outdoor_source_requests_ten_metre_radius() {
    let payload = encode_source_request("outdoor_gps", "source-test", "run", 1).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(value["arrival_radius_m"], 10.);
}

#[test]
fn temporary_heading_limit_accepts_35_degrees_but_keeps_freshness() {
    let mut report = ready();
    report.heading_accuracy_deg = Some(35.);
    assert!(admit_outdoor_goal(gate(Some(&report))).is_ok());
    report.heading_age_s = Some(2.1);
    assert_eq!(admit_outdoor_goal(gate(Some(&report))).unwrap_err(), "Heading uncertain");
}
