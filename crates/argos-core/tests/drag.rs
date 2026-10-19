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
    match drag.release() {
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
    assert!(matches!(drag.release(), DragEnd::Cancelled(_)));
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
fn geographic_drag_is_capability_gated_and_freezes_the_radius() {
    let mut gate = open_gate("waypoint");
    gate.geographic = true;
    assert_eq!(
        drag_disabled_reason(gate),
        Some("Outdoor GPS goals are unsupported on this rover")
    );
    gate.outdoor = true;
    gate.level = Some("waypoint_direct");
    assert_eq!(
        drag_disabled_reason(gate),
        Some("Outdoor goals need L3 Waypoint")
    );
    gate.level = Some("waypoint");
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
    drag.move_to(PlanarPoint {
        x: 38.8297 + 0.0003,
        y: -77.3075,
    });
    assert!(drag.distance_m() > 20.);
    match drag.release() {
        DragEnd::Send(GoalRequest::Geographic { latitude, .. }) => {
            assert!(latitude > 38.8297);
        }
        other => panic!("expected a geographic goal, got {other:?}"),
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
