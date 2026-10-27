use argos_core::*;

fn open_gate(level: &'static str) -> DragGate<'static> {
    DragGate {
        selected: true,
        link_healthy: true,
        pose_fresh: true,
        degraded: false,
        level: Some(level),
        geographic: false,
        outdoor: false,
        admission: None,
    }
}

fn local_drag() -> GoalDrag {
    GoalDrag::start(
        3,
        DragAnchor::Rover,
        DragSpace::Local,
        PlanarPoint { x: 1., y: 2. },
        PlanarPoint { x: 1., y: 2. },
        None,
        open_gate("waypoint"),
    )
    .unwrap()
}

#[test]
fn drag_starts_moves_and_release_sends_without_a_confirm() {
    let mut drag = local_drag();
    assert_eq!(drag.distance_m(), 0.);
    drag.move_to(PlanarPoint { x: 4., y: 6. });
    assert!((drag.distance_m() - 5.).abs() < 1e-9);
    match drag.release(open_gate("waypoint")) {
        DragEnd::Send(GoalRequest::Local { x, y, yaw: None }) => {
            assert_eq!((x, y), (4., 6.));
        }
        other => panic!("expected a local goal, got {other:?}"),
    }
}

#[test]
fn escape_cancels_and_a_drop_on_the_rover_sends_nothing() {
    let drag = local_drag();
    assert!(matches!(drag.escape(), DragEnd::Cancelled(_)));

    let mut drag = local_drag();
    drag.move_to(PlanarPoint {
        x: 1. + DROP_ON_SELF_METRES * 0.5,
        y: 2.,
    });
    assert!(matches!(
        drag.release(open_gate("waypoint")),
        DragEnd::Cancelled(_)
    ));
}

#[test]
fn a_stale_pose_refuses_to_start_a_drag() {
    let mut gate = open_gate("waypoint_direct");
    gate.pose_fresh = false;
    assert_eq!(drag_disabled_reason(gate), Some("Pose is stale"));
    assert!(
        GoalDrag::start(
            3,
            DragAnchor::Goal,
            DragSpace::Local,
            PlanarPoint { x: 0., y: 0. },
            PlanarPoint { x: 2., y: 0. },
            None,
            gate,
        )
        .is_err()
    );
}

#[test]
fn geographic_drag_release_sends_wgs84_and_freezes_the_radius() {
    let mut unsupported = open_gate("waypoint");
    unsupported.geographic = true;
    assert_eq!(
        drag_disabled_reason(unsupported),
        Some("Outdoor GPS goals are unsupported on this rover")
    );

    let report = ready_outdoor();
    let mut direct = geo_gate(&report, "waypoint_direct");
    if let Some(admission) = direct.admission.as_mut() {
        admission.level = Some("waypoint_direct");
    }
    assert_eq!(
        drag_disabled_reason(direct),
        Some("Outdoor GPS missions require L3 Waypoint")
    );

    // A Terra that lists goal_levels takes a GPS destination at L2–L5.
    let mut every = report.clone();
    every.goal_levels = ["waypoint_direct", "waypoint", "explore", "task"]
        .map(String::from)
        .to_vec();
    for level in ["waypoint_direct", "waypoint", "explore", "task"] {
        assert_eq!(
            drag_disabled_reason(geo_gate(&every, level)),
            None,
            "{level}"
        );
    }
    for level in ["teleop", "assisted_teleop", "target_search"] {
        assert_eq!(
            drag_disabled_reason(geo_gate(&every, level)),
            Some("GPS goals need L2 Direct, L3 Waypoint, L4 Explore or L5 Task"),
            "{level}"
        );
    }

    let mut stale = report.clone();
    stale.gps_age_s = Some(9.);
    assert_eq!(
        drag_disabled_reason(geo_gate(&stale, "waypoint")),
        Some("GPS stale")
    );
    let mut uncertain = report.clone();
    uncertain.heading_accuracy_deg = Some(40.);
    assert_eq!(
        drag_disabled_reason(geo_gate(&uncertain, "waypoint")),
        Some("Heading uncertain")
    );
    assert_eq!(
        drag_disabled_reason(geo_gate_report(None)),
        Some("App update required")
    );

    let gate = geo_gate(&report, "waypoint");
    let mut drag = GoalDrag::start(
        3,
        DragAnchor::Rover,
        DragSpace::Geographic,
        PlanarPoint {
            x: 38.8297,
            y: -77.3075,
        },
        PlanarPoint {
            x: 38.8297,
            y: -77.3075,
        },
        Some(5.),
        gate,
    )
    .unwrap();
    assert_eq!(drag.arrival_radius_m, Some(5.));
    assert!(matches!(drag.clone().escape(), DragEnd::Cancelled(_)));
    drag.move_to(PlanarPoint {
        x: 38.8297 + 0.000001,
        y: -77.3075,
    });
    assert!(matches!(
        drag.release(geo_gate(&report, "waypoint")),
        DragEnd::Cancelled(reason) if reason.contains("Dropped")
    ));

    let mut drag = GoalDrag::start(
        3,
        DragAnchor::Rover,
        DragSpace::Geographic,
        PlanarPoint {
            x: 38.8297,
            y: -77.3075,
        },
        PlanarPoint {
            x: 38.8297,
            y: -77.3075,
        },
        Some(5.),
        geo_gate(&report, "waypoint"),
    )
    .unwrap();
    let latitude = 38.8297 + 0.0003;
    let longitude = -77.3075;
    drag.move_to(PlanarPoint {
        x: latitude,
        y: longitude,
    });
    assert!(drag.distance_m() > 20.);
    match drag.release(geo_gate(&report, "waypoint")) {
        DragEnd::Send(goal) => match goal {
            GoalRequest::Geographic {
                latitude: sent_lat,
                longitude: sent_lon,
                yaw: None,
            } => {
                assert_eq!(sent_lat, latitude);
                assert_eq!(sent_lon, longitude);
                let body: serde_json::Value =
                    serde_json::from_slice(&goal.encode("goal-1").unwrap()).unwrap();
                assert_eq!(body["frame"], "wgs84");
                assert_eq!(body["latitude"], latitude);
                assert_eq!(body["longitude"], longitude);
                assert!(body.get("arrival_radius").is_none());
            }
            other => panic!("destination changed: {other:?}"),
        },
        other => panic!("expected a geographic goal, got {other:?}"),
    }
}

fn ready_outdoor() -> OutdoorReport {
    OutdoorReport::decode(&serde_json::json!({
        "schema_version": 1,
        "capability": "outdoor_gps",
        "supported": true,
        "acknowledged": true,
        "source": "outdoor_gps",
        "run_id": "run",
        "navigation_revision": 4,
        "ready": true,
        "gps_age_s": 0.4,
        "gps_accuracy_m": 4.0,
        "heading_age_s": 0.2,
        "heading_accuracy_deg": 8.0,
        "arrival_radius_m": 5.0,
        "gps": {"latitude": 38.8297, "longitude": -77.3075, "accuracy_m": 4.0, "age_s": 0.4},
        "age": 0.2
    }))
    .unwrap()
}

fn geo_gate<'a>(report: &'a OutdoorReport, level: &'static str) -> DragGate<'a> {
    let mut gate = geo_gate_report(Some(report));
    gate.level = Some(level);
    if let Some(admission) = gate.admission.as_mut() {
        admission.level = Some(level);
    }
    gate
}

fn geo_gate_report<'a>(report: Option<&'a OutdoorReport>) -> DragGate<'a> {
    DragGate {
        selected: true,
        link_healthy: true,
        pose_fresh: true,
        degraded: false,
        level: Some("waypoint"),
        geographic: true,
        outdoor: report.is_some_and(|report| report.acknowledged_outdoor()),
        admission: Some(OutdoorAdmission {
            report,
            latitude: 38.8297,
            longitude: -77.3075,
            arrival_radius_m: 5.,
            level: Some("waypoint"),
            run_id: "run",
            navigation_revision: 4,
        }),
    }
}

#[test]
fn degraded_pose_blocks_the_drag_and_phone_manual_hides_the_warning() {
    let mut gate = open_gate("waypoint");
    gate.degraded = true;
    assert_eq!(
        drag_disabled_reason(gate),
        Some("pose is degraded; not sending a goal")
    );
    assert!(pose_warning_text(CommandOrigin::PhoneBluetooth, true, "tracking lost").is_none());
    let shown = pose_warning_text(CommandOrigin::FleetTeleop, true, "tracking lost").unwrap();
    assert!(shown.contains("informational"));
    assert!(goal_blocked(true).is_some());
}
