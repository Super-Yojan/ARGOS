use argos_server::{
    engine::{Engine, Fleet},
    types::*,
};
#[derive(Default)]
struct Vehicle {
    sent: Vec<(f64, f64)>,
    stale: bool,
    revision: bool,
    unarmed: bool,
    sequences: Vec<u64>,
}
impl Fleet for Vehicle {
    fn snapshot(&mut self, _: f64) -> FleetSnapshot {
        let mut r = Rover::demo("1", 0., 0.);
        r.authority.as_mut().unwrap().requested_level = "teleop".into();
        if self.unarmed {
            r.hardware.as_mut().unwrap().armed = false;
        }
        if self.stale {
            r.pose_age = Some(1.);
        }
        if self.revision {
            r.authority.as_mut().unwrap().revision = "2".into();
        }
        FleetSnapshot {
            link: "healthy".into(),
            rovers: vec![r],
            ..Default::default()
        }
    }
    fn select_mode(&mut self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn send_goal(&mut self, _: &str, _: &Waypoint) -> Result<String, String> {
        unreachable!()
    }
    fn cancel(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn drive(&mut self, _: &str, r: &DriveRequest, _: &str) -> Result<(), String> {
        self.sent.push((r.linear, r.angular));
        self.sequences.push(r.sequence);
        Ok(())
    }
}
fn user() -> Identity {
    Identity {
        id: "u".into(),
        login: "u".into(),
        role: "operator".into(),
        csrf: "".into(),
    }
}
fn req(sequence: u64, at: f64) -> DriveRequest {
    DriveRequest {
        run_id: "demo-run".into(),
        revision: "1".into(),
        sequence,
        observed_at: at,
        linear: 0.3,
        angular: 0.,
    }
}
#[test]
fn lease_expires_and_cannot_replay_older_pulses() {
    let mut e = Engine::new(Vehicle::default());
    e.claim(&user(), "tab", 0.).unwrap();
    e.drive(&user(), "tab", "1", req(1, 0.), 0.).unwrap();
    e.tick(0.5);
    assert_eq!(e.fleet.sent, vec![(0.3, 0.), (0., 0.)]);
    assert!(e.drive(&user(), "tab", "1", req(1, 0.5), 0.5).is_err());
    assert!(e.drive(&user(), "tab", "1", req(2, 0.), 1.).is_err());
}
#[test]
fn claim_loss_and_changed_authority_stop_motion() {
    let mut e = Engine::new(Vehicle::default());
    e.claim(&user(), "tab", 0.).unwrap();
    e.drive(&user(), "tab", "1", req(1, 0.), 0.).unwrap();
    e.release("tab");
    assert_eq!(e.fleet.sent.last(), Some(&(0., 0.)));
    assert!(e.drive(&user(), "tab", "1", req(2, 0.), 0.).is_err());
    e.claim(&user(), "tab", 0.).unwrap();
    e.drive(&user(), "tab", "1", req(3, 0.1), 0.1).unwrap();
    e.fleet.revision = true;
    e.tick(0.2);
    assert_eq!(e.fleet.sent.last(), Some(&(0., 0.)));
}
#[test]
fn stale_unarmed_and_wrong_revision_fail_closed() {
    let mut e = Engine::new(Vehicle::default());
    e.claim(&user(), "tab", 0.).unwrap();
    e.fleet.stale = true;
    assert!(e.drive(&user(), "tab", "1", req(1, 0.), 0.).is_err());
    e.fleet.stale = false;
    e.fleet.revision = true;
    assert!(e.drive(&user(), "tab", "1", req(2, 0.), 0.).is_err());
    e.fleet.revision = false;
    e.fleet.unarmed = true;
    assert!(e.drive(&user(), "tab", "1", req(3, 0.), 0.).is_err());
    assert!(e.fleet.sent.is_empty());
}
#[test]
fn emergency_stop_revokes_claim_and_latches_demo_safety() {
    let mut e = Engine::new(argos_server::fleet::DemoFleet::default());
    e.claim(&user(), "tab", 0.).unwrap();
    e.emergency_stop(&user(), "tab", "1", 0.).unwrap();
    assert!(e.claim.is_none());
    assert_eq!(e.fleet.rovers[0].authority.as_ref().unwrap().safety, "stop");
    assert!(!e.fleet.rovers[0].hardware.as_ref().unwrap().armed);
    assert!(e.drive(&user(), "tab", "1", req(1, 0.), 0.).is_err());
}

#[test]
fn release_outranks_delayed_press_and_wire_sequences_survive_new_tabs() {
    let mut e = Engine::new(Vehicle::default());
    e.claim(&user(), "tab", 0.).unwrap();
    e.drive(&user(), "tab", "1", req(1, 0.), 0.).unwrap();
    let mut neutral = req(3, 0.1);
    neutral.linear = 0.;
    e.drive(&user(), "tab", "1", neutral, 0.1).unwrap();
    assert!(e.drive(&user(), "tab", "1", req(2, 0.1), 0.2).is_err());
    assert_eq!(e.fleet.sent.last(), Some(&(0., 0.)));
    e.release("tab");
    e.claim(&user(), "other", 0.3).unwrap();
    e.drive(&user(), "other", "1", req(1, 0.3), 0.3).unwrap();
    e.tick(1.);
    assert!(e.fleet.sequences.windows(2).all(|s| s[1] > s[0]));
}
#[tokio::test]
async fn worker_rejects_motion_that_expired_waiting_in_queue() {
    let store = argos_server::store::Store::open("sqlite::memory:")
        .await
        .unwrap();
    let worker = argos_server::worker::Worker::start(Box::new(Vehicle::default()), store)
        .await
        .unwrap();
    worker
        .request(user(), "tab".into(), argos_server::worker::Action::Claim)
        .await
        .unwrap();
    let result = worker
        .request(
            user(),
            "tab".into(),
            argos_server::worker::Action::Drive(
                "1".into(),
                req(1, 0.),
                std::time::Instant::now() - std::time::Duration::from_secs(1),
            ),
        )
        .await;
    assert_eq!(result.unwrap_err(), "Drive input expired in queue");
    worker.stop();
}
#[test]
fn selecting_manual_mode_does_not_reset_emergency_latch() {
    let mut fleet = argos_server::fleet::DemoFleet::default();
    fleet.emergency_stop("1").unwrap();
    fleet.select_mode("1", "teleop", "mode").unwrap();
    fleet.snapshot(1.);
    assert_eq!(fleet.rovers[0].authority.as_ref().unwrap().safety, "stop");
}
#[test]
fn emergency_stop_is_audited_without_storage_blocking_safety() {
    let mut e = Engine::new(argos_server::fleet::DemoFleet::default());
    e.claim(&user(), "tab", 0.).unwrap();
    e.emergency_stop(&user(), "tab", "1", 0.).unwrap();
    let stop = e
        .operations
        .values()
        .find(|o| o.kind == "stop")
        .expect("stop audit");
    assert_eq!(stop.phase, "unconfirmed");
    assert!(stop.message.contains("Published"));
    e.claim(&user(), "tab", 1.).unwrap();
    e.journal(Box::new(|_| Err("disk failed".into())));
    assert!(e.emergency_stop(&user(), "tab", "2", 1.).is_err());
    assert_eq!(e.fleet.rovers[1].authority.as_ref().unwrap().safety, "stop");
    assert!(!e.fleet.rovers[1].hardware.as_ref().unwrap().armed);
}
