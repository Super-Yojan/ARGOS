use argos_server::{engine::Engine, fleet::DemoFleet, types::*};
fn user() -> Identity {
    Identity {
        id: "1".into(),
        login: "test".into(),
        role: "operator".into(),
        csrf: "c".into(),
    }
}
fn req(level: &str) -> AutonomyRequest {
    AutonomyRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        level: level.into(),
        run_id: "demo-run".into(),
        revision: "1".into(),
    }
}
#[test]
fn disarmed_held_rover_can_request_direct_mode_without_arming_or_replaying() {
    let mut e = Engine::new(DemoFleet::default());
    e.claim(&user(), "tab", 0.).unwrap();
    e.fleet.rovers[1].authority.as_mut().unwrap().safety = "hold".into();
    let request = req("waypoint_direct");
    let op = e
        .autonomy(&user(), "tab", "2", request.clone(), 0.)
        .unwrap();
    assert_eq!(op.phase, "autonomy_pending");
    e.tick(1.);
    assert_eq!(e.operations[&op.id].phase, "confirmed");
    assert!(!e.fleet.rovers[1].hardware.as_ref().unwrap().armed);
    assert_eq!(
        e.autonomy(&user(), "tab", "2", request, 1.).unwrap().id,
        op.id
    );
}
#[test]
fn mode_selection_requires_control_fresh_current_authority_and_supported_level() {
    let mut e = Engine::new(DemoFleet::default());
    assert!(
        e.autonomy(&user(), "tab", "2", req("waypoint_direct"), 0.)
            .is_err()
    );
    e.claim(&user(), "tab", 0.).unwrap();
    assert!(
        e.autonomy(&user(), "tab", "2", req("decision_making"), 0.)
            .is_err()
    );
    let mut r = req("waypoint_direct");
    r.run_id = "old".into();
    assert!(e.autonomy(&user(), "tab", "2", r, 0.).is_err());
    e.fleet.rovers[1].authority.as_mut().unwrap().age = 0.5;
    assert!(
        e.autonomy(&user(), "tab", "2", req("waypoint_direct"), 0.)
            .is_err()
    );
}
#[test]
fn authority_change_during_journaling_cannot_publish_a_mode() {
    use argos_server::engine::Fleet;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    struct Shared {
        fleet: Arc<Mutex<DemoFleet>>,
        sent: Arc<AtomicUsize>,
    }
    impl Fleet for Shared {
        fn snapshot(&mut self, now: f64) -> FleetSnapshot {
            self.fleet.lock().unwrap().snapshot(now)
        }
        fn select_mode(&mut self, _: &str, _: &str, _: &str) -> Result<(), String> {
            self.sent.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn send_goal(&mut self, _: &str, _: &Waypoint) -> Result<String, String> {
            unreachable!()
        }
        fn cancel(&mut self, _: &str) -> Result<(), String> {
            unreachable!()
        }
    }
    let fleet = Arc::new(Mutex::new(DemoFleet::default()));
    let sent = Arc::new(AtomicUsize::new(0));
    let mut e = Engine::new(Shared {
        fleet: fleet.clone(),
        sent: sent.clone(),
    });
    e.claim(&user(), "tab", 0.).unwrap();
    e.journal(Box::new(move |_| {
        fleet.lock().unwrap().rovers[1]
            .authority
            .as_mut()
            .unwrap()
            .revision = "2".into();
        Ok(())
    }));
    let op = e
        .autonomy(&user(), "tab", "2", req("waypoint_direct"), 0.)
        .unwrap();
    assert_eq!(op.phase, "interrupted");
    assert_eq!(sent.load(Ordering::SeqCst), 0);
}
