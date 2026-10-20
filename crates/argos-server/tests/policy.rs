use argos_server::{
    engine::{Engine, Fleet},
    types::*,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Calls {
    modes: Vec<String>,
    goals: usize,
    goal: Option<Waypoint>,
    sources: Vec<String>,
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
    fn send_goal(&mut self, _: &str, goal: &Waypoint) -> Result<String, String> {
        let mut calls = self.calls.lock().unwrap();
        calls.goals += 1;
        calls.goal = Some(goal.clone());
        Ok("goal-token".into())
    }
    fn select_source(
        &mut self,
        _: &str,
        source: &str,
        token: &str,
        run_id: &str,
        navigation_revision: &str,
    ) -> Result<(), String> {
        self.calls.lock().unwrap().sources.push(source.into());
        if let Some(outdoor) = self.rover.outdoor.as_mut() {
            outdoor.source = source.into();
            outdoor.acknowledged = source == "outdoor_gps";
            outdoor.token = Some(token.into());
            outdoor.result = Some("accepted".into());
            outdoor.run_id = run_id.into();
            outdoor.navigation_revision =
                (navigation_revision.parse::<u64>().unwrap() + 1).to_string();
            outdoor.age = 0.;
        }
        if self
            .rover
            .hardware
            .as_ref()
            .is_some_and(|hardware| hardware.armed)
        {
            return Err("armed".into());
        }
        Ok(())
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
        arrival_radius: None,
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
    e.fleet.rover.reference = Some(Reference {
        latitude: 38.8297,
        longitude: -77.3075,
        anchor_x: 0.,
        anchor_y: 0.,
        rotation: 0.,
        frame_id: "demo-run".into(),
    });
    let result = e.submit(&who(), "tab", ID, req, 0.);
    let error = result.unwrap_err();
    assert!(
        error.contains("App update required"),
        "an alignment must not authorize geographic driving: {error}"
    );
    assert!(c.lock().unwrap().modes.is_empty());
    assert_eq!(c.lock().unwrap().goals, 0);
}

fn outdoor_status() -> OutdoorStatus {
    OutdoorStatus::from_report(
        &argos_core::OutdoorReport::decode(&serde_json::json!({
            "schema_version": 1,
            "capability": "outdoor_gps",
            "supported": true,
            "acknowledged": true,
            "source": "outdoor_gps",
            "run_id": "demo-run",
            "navigation_revision": 4,
            "ready": true,
            "gps_age_s": 0.4,
            "gps_accuracy_m": 4.0,
            "heading_age_s": 0.2,
            "heading_accuracy_deg": 8.0,
            "arrival_radius_m": 5.0,
            "gps": {"latitude": 38.8297, "longitude": -77.3075},
            "phase": "idle",
            "age": 0.1
        }))
        .unwrap(),
        0.1,
    )
}

#[test]
fn geographic_goal_publishes_wgs84_only_after_fresh_outdoor_readiness() {
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    e.fleet.rover.outdoor = Some(outdoor_status());
    e.fleet.rover.authority.as_mut().unwrap().requested_level = "waypoint".into();
    let mut req = request();
    req.arrival_radius = Some(5.);
    req.waypoint = Waypoint::Geographic {
        latitude: 38.8299,
        longitude: -77.3075,
        yaw: None,
    };
    let op = e.submit(&who(), "tab", ID, req, 0.).unwrap();
    assert_eq!(e.operations[&op.id].arrival_radius, Some(5.));
    assert_eq!(
        e.operations[&op.id].navigation_revision.as_deref(),
        Some("4")
    );
    e.tick(1.);
    assert_eq!(c.lock().unwrap().goals, 1);
    match c.lock().unwrap().goal.clone().unwrap() {
        Waypoint::Geographic {
            latitude,
            longitude,
            ..
        } => {
            assert_eq!(latitude, 38.8299);
            assert_eq!(longitude, -77.3075);
        }
        other => panic!("destination was rewritten: {other:?}"),
    }
    assert_eq!(e.operations[&op.id].phase, "published");
}

#[test]
fn stale_outdoor_readiness_is_rechecked_and_does_not_publish() {
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    e.fleet.rover.outdoor = Some(outdoor_status());
    e.fleet.rover.authority.as_mut().unwrap().requested_level = "teleop".into();
    let mut req = request();
    req.request_id = "4d0c6a2e-1c3b-4f4a-9a1e-0c2b6d7e8f90".into();
    req.waypoint = Waypoint::Geographic {
        latitude: 38.8299,
        longitude: -77.3075,
        yaw: None,
    };
    let op = e.submit(&who(), "tab", ID, req, 0.).unwrap();
    e.fleet.rover.outdoor.as_mut().unwrap().gps_age_s = Some(9.);
    let token = c.lock().unwrap().modes[0].clone();
    let authority = e.fleet.rover.authority.as_mut().unwrap();
    authority.token = Some(token);
    authority.result = Some("accepted".into());
    authority.requested_level = "waypoint".into();
    e.tick(1.);
    assert_eq!(c.lock().unwrap().goals, 0);
    assert_eq!(e.operations[&op.id].phase, "interrupted");
    assert!(e.operations[&op.id].message.contains("GPS stale"));
}

#[test]
fn direct_level_and_unarmed_source_changes_fail_closed() {
    let (mut e, c) = setup();
    e.claim(&who(), "tab", 0.).unwrap();
    e.fleet.rover.outdoor = Some(outdoor_status());
    e.fleet.rover.authority.as_mut().unwrap().requested_level = "waypoint_direct".into();
    let mut req = request();
    req.waypoint = Waypoint::Geographic {
        latitude: 38.8299,
        longitude: -77.3075,
        yaw: None,
    };
    assert!(
        e.submit(&who(), "tab", ID, req, 0.)
            .unwrap_err()
            .contains("L3")
    );
    e.fleet.rover.hardware.as_mut().unwrap().armed = true;
    let armed = e
        .select_source(
            &who(),
            "tab",
            ID,
            SourceRequest {
                request_id: "6b1e2f44-0a55-4d2c-9c11-2a0b7e6d5c31".into(),
                run_id: "demo-run".into(),
                revision: "1".into(),
                navigation_revision: "4".into(),
                source: "local".into(),
            },
            1.,
        )
        .unwrap_err();
    assert!(armed.contains("disarmed"), "{armed}");
    assert!(c.lock().unwrap().sources.is_empty());
    assert!(e.fleet.rover.hardware.as_ref().unwrap().armed);
    e.fleet.rover.hardware.as_mut().unwrap().armed = false;
    e.fleet.rover.hardware.as_mut().unwrap().arming = Some(false);
    let op = e
        .select_source(
            &who(),
            "tab",
            ID,
            SourceRequest {
                request_id: "7c2f3044-1b66-4e3d-8d22-3b1c8f7e6d42".into(),
                run_id: "demo-run".into(),
                revision: "1".into(),
                navigation_revision: "4".into(),
                source: "outdoor_gps".into(),
            },
            2.,
        )
        .unwrap();
    assert_eq!(c.lock().unwrap().sources, vec!["outdoor_gps".to_string()]);
    assert!(!e.fleet.rover.hardware.as_ref().unwrap().armed);
    e.tick(3.);
    assert_eq!(e.operations[&op.id].phase, "confirmed");
}
