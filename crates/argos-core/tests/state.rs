use argos_core::*;
fn cache() -> FleetCache {
    let mut c = FleetCache::default();
    c.fleet(
        decode_fleet(br#"{"count":2,"max_count":32,"ids":[0,8]}"#).unwrap(),
        0.,
    );
    c
}
fn status(token: &str, state: &str) -> GoalStatus {
    decode_status(format!("{{\"state\":\"{state}\",\"goal_id\":1,\"token\":\"{token}\",\"distance\":1,\"x\":12,\"y\":0}}").as_bytes()).unwrap()
}
#[test]
fn correlates_only_current_token_and_never_retries() {
    let mut c = cache();
    let request = GoalRequest::Local {
        x: 12.,
        y: 0.,
        yaw: None,
    };
    assert!(c.prepare_goal(0, &request, "new", 0.1).is_ok());
    assert!(c.prepare_goal(0, &request, "duplicate", 0.2).is_err());
    c.status(0, status("old", "arrived"), 0.3);
    assert_eq!(c.snapshot(0.3).rovers[0].command_phase, "pending");
    c.status(0, status("new", "active"), 0.4);
    assert_eq!(c.snapshot(0.4).rovers[0].command_phase, "active");
    c.status(0, status("new", "arrived"), 0.5);
    assert_eq!(c.snapshot(0.5).rovers[0].command_phase, "arrived");
}
#[test]
fn stale_and_removed_members_cannot_receive_commands() {
    let mut c = cache();
    let r = GoalRequest::Local {
        x: 1.,
        y: 0.,
        yaw: None,
    };
    assert!(c.prepare_goal(8, &r, "a", SUPERVISION_STALE_SECONDS).is_err());
    assert!(c.prepare_goal(2, &r, "b", 0.1).is_err());
    c.fleet(
        decode_fleet(br#"{"count":1,"max_count":32,"ids":[0]}"#).unwrap(),
        0.2,
    );
    assert!(c.prepare_goal(8, &r, "c", 0.3).is_err());
    assert_eq!(c.snapshot(0.3).rovers[1].membership, "absent");
}
#[test]
fn freshness_is_independent_and_cancel_requires_later_idle() {
    let mut c = cache();
    c.pose(
        0,
        Pose {
            rover_id: 0,
            sequence: 1,
            x: 0.,
            y: 0.,
            yaw: 0.,
        },
        0.1,
    );
    c.status(0, status("other", "active"), 0.1);
    c.prepare_cancel(0, 0.2).unwrap();
    assert_eq!(c.snapshot(0.2).rovers[0].command_phase, "cancelling");
    c.status(
        0,
        decode_status(br#"{"state":"idle","goal_id":0,"distance":0,"x":0,"y":0}"#).unwrap(),
        0.3,
    );
    assert_eq!(c.snapshot(0.3).rovers[0].command_phase, "cancelled");
    c.fleet(
        decode_fleet(br#"{"count":2,"max_count":32,"ids":[0,8]}"#).unwrap(),
        3.,
    );
    let s = c.snapshot(3.);
    assert_eq!(s.link, "healthy");
    assert!(s.rovers[0].pose_age.unwrap() >= 2.5);
}
#[test]
fn pending_timeout_and_disconnect_do_not_replay() {
    let mut c = cache();
    c.prepare_goal(
        0,
        &GoalRequest::Local {
            x: 1.,
            y: 0.,
            yaw: None,
        },
        "a",
        0.1,
    )
    .unwrap();
    assert_eq!(c.snapshot(GOAL_ACK_SECONDS + 0.1).rovers[0].command_phase, "unconfirmed");
    c.disconnected();
    assert_eq!(c.snapshot(0.2).link, "disconnected");
    assert!(c.prepare_cancel(0, 0.2).is_err());
}
#[test]
fn pose_id_mismatch_and_unlisted_status_are_ignored() {
    let mut c = cache();
    c.pose(
        0,
        Pose {
            rover_id: 8,
            sequence: 0,
            x: 3.,
            y: 0.,
            yaw: 0.,
        },
        0.1,
    );
    c.status(99, status("a", "active"), 0.1);
    assert!(c.snapshot(0.1).rovers[0].pose.is_none());
    assert_eq!(c.snapshot(0.1).rovers.len(), 2);
}

#[test]
fn retired_rovers_do_not_grow_snapshots_without_bound() {
    let mut c = FleetCache::default();
    for id in 0..100 {
        c.fleet(
            FleetState {
                count: 1,
                max_count: 32,
                ids: vec![id],
            },
            id as f64,
        );
    }
    let snapshot = c.snapshot(99.);
    assert!(snapshot.rovers.len() <= 33);
    assert!(
        snapshot
            .rovers
            .iter()
            .any(|r| r.id == 99 && r.membership == "online")
    );
}

#[test]
fn forest_gap_accepts_waypoint_and_delayed_ack_without_retry() {
    let mut c = cache();
    let goal = GoalRequest::Local { x: 10., y: 20., yaw: None };
    assert_eq!(c.snapshot(60.).link, "healthy");
    c.prepare_goal(8, &goal, "forest", 60.).unwrap();
    assert_eq!(c.snapshot(120.).rovers[1].command_phase, "pending");
    c.status(8, status("forest", "arrived"), 180.);
    assert_eq!(c.snapshot(180.).rovers[1].command_phase, "arrived");
    assert!(c.prepare_goal(8, &goal, "expired", 90.).is_err());
}
