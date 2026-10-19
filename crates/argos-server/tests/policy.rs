use argos_server::{
    engine::{Engine, Fleet},
    types::*,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Calls {
    modes: Vec<String>,
    goals: usize,
    cancels: usize,
}
struct Fake {
    rover: Rover,
    calls: Arc<Mutex<Calls>>,
}
impl Fleet for Fake {
    fn snapshot(&mut self, _: f64) -> FleetSnapshot {
        FleetSnapshot {
            link: "healthy".into(),
            rovers: vec![self.rover.clone()],
            ..Default::default()
        }
    }
    fn select_mode(&mut self, _: &str, _: &str, token: &str) -> Result<(), String> {
        self.calls.lock().unwrap().modes.push(token.into());
        Ok(())
    }
    fn send_goal(&mut self, _: &str, _: &Waypoint) -> Result<String, String> {
        self.calls.lock().unwrap().goals += 1;
        Ok("goal-token".into())
    }
    fn cancel(&mut self, _: &str) -> Result<(), String> {
        self.calls.lock().unwrap().cancels += 1;
        Ok(())
    }
}
fn setup() -> (Engine<Fake>, Arc<Mutex<Calls>>) {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let f = Fake {
        rover: Rover::demo("18446744073709551615", 0., 0.),
        calls: calls.clone(),
    };
    (Engine::new(f), calls)
}
fn request() -> WaypointRequest {
    WaypointRequest {
        run_id: "demo-run".into(),
        frame_id: Some("demo-run".into()),
        request_id: "c70eb2dc-7855-441c-825f-7ac55858befe".into(),
        waypoint: Waypoint::Local {
            x: 2.,
            y: 3.,
            yaw: None,
        },
    }
}
fn who() -> Identity {
    Identity {
        id: "1".into(),
        login: "operator".into(),
        role: "operator".into(),
        csrf: "csrf".into(),
    }
}
const ID: &str = "18446744073709551615";
#[test]
fn waits_for_matching_mode_then_publishes_once() {
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    let op = e.submit(&who(), "tab", ID, request(), 0.).unwrap();
    e.tick(1.);
    assert_eq!(c.lock().unwrap().goals, 0);
    let t = c.lock().unwrap().modes[0].clone();
    let a = e.fleet.rover.authority.as_mut().unwrap();
    a.token = Some(t);
    a.result = Some("accepted".into());
    a.requested_level = "waypoint".into();
    e.tick(2.);
    e.tick(3.);
    assert_eq!(c.lock().unwrap().goals, 1);
    assert_eq!(e.operations[&op.id].phase, "published");
}
#[test]
fn duplicate_request_does_not_repeat_mode_or_goal_and_changed_payload_conflicts() {
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    let first = e.submit(&who(), "tab", ID, request(), 0.).unwrap();
    assert_eq!(
        first.id,
        e.submit(&who(), "tab", ID, request(), 0.).unwrap().id
    );
    assert_eq!(c.lock().unwrap().modes.len(), 1);
    let mut changed = request();
    changed.waypoint = Waypoint::Local {
        x: 9.,
        y: 1.,
        yaw: None,
    };
    assert!(e.submit(&who(), "tab", ID, changed, 0.).is_err());
}
#[test]
fn rejects_disarmed_stale_missing_and_viewer_commands() {
    let (mut e, _) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    e.fleet.rover.hardware.as_mut().unwrap().armed = false;
    assert!(e.submit(&who(), "tab", ID, request(), 0.).is_err());
    e.fleet.rover.hardware.as_mut().unwrap().armed = true;
    e.fleet.rover.pose_age = Some(90.);
    assert!(e.submit(&who(), "tab", ID, request(), 0.).is_err());
    e.fleet.rover.pose_age = Some(0.);
    e.fleet.rover.pose = None;
    assert!(e.submit(&who(), "tab", ID, request(), 0.).is_err());
    let mut viewer = who();
    viewer.role = "viewer".into();
    assert!(e.claim(&viewer, "other", 0.).is_err());
}
#[test]
fn losing_claim_or_readiness_never_publishes_pending_goal() {
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    let op = e.submit(&who(), "tab", ID, request(), 0.).unwrap();
    e.tick(31.);
    assert_eq!(e.operations[&op.id].phase, "interrupted");
    assert_eq!(c.lock().unwrap().goals, 0);
    e.claim(&who(), "other", 32.).unwrap();
    e.tick(33.);
    assert_eq!(c.lock().unwrap().goals, 0);
}
#[test]
fn cancellation_supersedes_pending_mode_and_does_not_send_goal() {
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    let op = e.submit(&who(), "tab", ID, request(), 0.).unwrap();
    e.cancel(
        &who(),
        "tab",
        ID,
        "9f9d547d-e008-4a75-b206-784d88538f08",
        1.,
    )
    .unwrap();
    e.tick(2.);
    assert_eq!(e.operations[&op.id].phase, "cancelled");
    assert_eq!(c.lock().unwrap().goals, 0);
    assert_eq!(c.lock().unwrap().cancels, 1);
}
#[test]
fn competing_tabs_cannot_renew_or_release_each_others_claim() {
    let (mut e, _) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    assert!(e.claim(&who(), "other", 1.).is_err());
    e.release("other");
    assert!(e.claim(&who(), "other", 2.).is_err());
    e.release("tab");
    assert!(e.claim(&who(), "other", 3.).is_ok());
}
#[test]
fn readiness_is_rechecked_and_mode_timeout_never_retries() {
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    let op = e.submit(&who(), "tab", ID, request(), 0.).unwrap();
    e.fleet.rover.hardware.as_mut().unwrap().ready = false;
    e.tick(1.);
    assert_eq!(e.operations[&op.id].phase, "interrupted");
    assert_eq!(c.lock().unwrap().goals, 0);
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    let op = e.submit(&who(), "tab", ID, request(), 0.).unwrap();
    for t in [20., 40., 60., 80., 100., 120., 140., 151.] {
        e.claim(&who(), "tab", t).unwrap();
        e.tick(t);
    }
    assert_eq!(e.operations[&op.id].phase, "unconfirmed");
    assert_eq!(c.lock().unwrap().modes.len(), 1);
    assert_eq!(c.lock().unwrap().goals, 0);
}
#[test]
fn rejects_unsupported_and_mismatched_mode_acknowledgments() {
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    let op = e.submit(&who(), "tab", ID, request(), 0.).unwrap();
    let a = e.fleet.rover.authority.as_mut().unwrap();
    a.token = Some(c.lock().unwrap().modes[0].clone());
    a.result = Some("rejected".into());
    e.tick(1.);
    assert_eq!(e.operations[&op.id].phase, "rejected");
    assert_eq!(c.lock().unwrap().goals, 0);
    let (mut e, _) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    e.fleet
        .rover
        .authority
        .as_mut()
        .unwrap()
        .supported_levels
        .clear();
    assert!(e.submit(&who(), "tab", ID, request(), 0.).is_err());
}
#[test]
fn ready_mode_is_rechecked_if_another_client_changes_authority() {
    let (mut e, c) = setup();
    e.fleet.rover.authority.as_mut().unwrap().requested_level = "waypoint".into();
    e.claim(&who(), "tab", 0.).unwrap();
    let op = e.submit(&who(), "tab", ID, request(), 0.).unwrap();
    e.fleet.rover.authority.as_mut().unwrap().requested_level = "teleop".into();
    e.tick(1.);
    assert_eq!(c.lock().unwrap().goals, 0);
    assert_eq!(e.operations[&op.id].phase, "interrupted");
}
#[test]
fn journal_failure_prevents_publication() {
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    e.journal(Box::new(|_| Err("disk full".into())));
    assert!(e.submit(&who(), "tab", ID, request(), 0.).is_err());
    e.tick(1.);
    assert_eq!(c.lock().unwrap().goals, 0);
    assert!(c.lock().unwrap().modes.is_empty());
}
#[test]
fn release_and_reacquire_cannot_revive_pending_intent() {
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    let op = e.submit(&who(), "tab", ID, request(), 0.).unwrap();
    e.release("tab");
    e.claim(&who(), "tab", 1.).unwrap();
    let a = e.fleet.rover.authority.as_mut().unwrap();
    a.token = Some(c.lock().unwrap().modes[0].clone());
    a.result = Some("accepted".into());
    a.requested_level = "waypoint".into();
    e.tick(2.);
    assert_eq!(e.operations[&op.id].phase, "interrupted");
    assert_eq!(c.lock().unwrap().goals, 0);
}
#[test]
fn renewal_after_expiry_cannot_revive_pending_intent() {
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    let op = e.submit(&who(), "tab", ID, request(), 0.).unwrap();
    e.claim(&who(), "tab", 31.).unwrap();
    let a = e.fleet.rover.authority.as_mut().unwrap();
    a.token = Some(c.lock().unwrap().modes[0].clone());
    a.result = Some("accepted".into());
    a.requested_level = "waypoint".into();
    e.tick(32.);
    assert_eq!(e.operations[&op.id].phase, "interrupted");
    assert_eq!(c.lock().unwrap().goals, 0);
}
#[test]
fn accepted_operation_ends_when_another_controller_cancels_it() {
    let (mut e, _) = setup();
    e.fleet.rover.authority.as_mut().unwrap().requested_level = "waypoint".into();
    e.claim(&who(), "tab", 0.).unwrap();
    let op = e.submit(&who(), "tab", ID, request(), 0.).unwrap();
    e.tick(1.);
    e.fleet.rover.goal = Some(Goal {
        state: "active".into(),
        token: Some("goal-token".into()),
        x: 2.,
        y: 3.,
        distance: 1.,
    });
    e.fleet.rover.goal_age = Some(0.);
    e.tick(2.);
    assert_eq!(e.operations[&op.id].phase, "accepted");
    e.fleet.rover.goal.as_mut().unwrap().state = "idle".into();
    e.fleet.rover.goal.as_mut().unwrap().token = None;
    e.tick(3.);
    assert!(e.operations[&op.id].terminal());
}
#[test]
fn local_frame_changes_invalidate_reviewed_requests_and_pending_goals() {
    let (mut e, c) = setup();
    e.fleet.rover.reference = None;
    e.claim(&who(), "tab", 0.).unwrap();
    e.fleet.rover.frame_id = Some("new-local-frame".into());
    assert!(e.submit(&who(), "tab", ID, request(), 0.).is_err());
    let (mut e, c2) = setup();
    e.fleet.rover.reference = None;
    e.claim(&who(), "tab", 0.).unwrap();
    let op = e.submit(&who(), "tab", ID, request(), 0.).unwrap();
    e.fleet.rover.frame_id = Some("new-local-frame".into());
    let a = e.fleet.rover.authority.as_mut().unwrap();
    a.token = Some(c2.lock().unwrap().modes[0].clone());
    a.result = Some("accepted".into());
    a.requested_level = "waypoint".into();
    e.tick(1.);
    assert_eq!(e.operations[&op.id].phase, "interrupted");
    assert_eq!(c.lock().unwrap().goals, 0);
    assert_eq!(c2.lock().unwrap().goals, 0);
}
#[test]
fn cancellation_requires_a_post_request_idle_observation() {
    let (mut e, _) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    e.fleet.rover.goal = Some(Goal {
        state: "idle".into(),
        token: None,
        x: 0.,
        y: 0.,
        distance: 0.,
    });
    e.fleet.rover.goal_age = Some(0.);
    e.fleet.rover.goal_observation = Some("old".into());
    let op = e
        .cancel(
            &who(),
            "tab",
            ID,
            "59a49d88-ac1d-4522-9361-1ef585fa9d37",
            1.,
        )
        .unwrap();
    e.tick(2.);
    assert_eq!(e.operations[&op.id].phase, "cancelling");
    e.fleet.rover.goal_observation = Some("new".into());
    e.tick(3.);
    assert_eq!(e.operations[&op.id].phase, "cancelled");
}
#[test]
fn geographic_commands_are_rejected_even_with_an_old_alignment() {
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    let mut req = request();
    req.waypoint = Waypoint::Geographic {
        latitude: 38.8298,
        longitude: -77.3075,
        yaw: None,
    };
    let result = e.submit(&who(), "tab", ID, req, 0.);
    assert!(
        result.is_err(),
        "GPS display must not authorize geographic driving"
    );
    assert!(c.lock().unwrap().modes.is_empty());
    assert_eq!(c.lock().unwrap().goals, 0);
}
