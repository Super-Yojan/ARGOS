use argos_server::{engine::Engine, fleet::DemoFleet, types::*};
fn who() -> Identity {
    Identity {
        id: "1".into(),
        login: "operator".into(),
        role: "operator".into(),
        csrf: "c".into(),
    }
}
fn request(action: &str) -> HardwareRequest {
    HardwareRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        action: action.into(),
        run_id: "demo-run".into(),
        revision: "1".into(),
    }
}
#[test]
fn disarmed_rover_can_be_armed_once_and_only_matching_receipt_confirms() {
    let mut e = Engine::new(DemoFleet::default());
    e.claim(&who(), "tab", 0.).unwrap();
    let r = request("arm");
    let op = e.hardware(&who(), "tab", "2", r.clone(), 0.).unwrap();
    assert_eq!(op.phase, "hardware_pending");
    e.fleet.rovers[1].hardware.as_mut().unwrap().token = Some("unrelated".into());
    e.tick(0.1);
    assert_eq!(e.operations[&op.id].phase, "hardware_pending");
    assert_eq!(e.hardware(&who(), "tab", "2", r, 0.2).unwrap().id, op.id);
    assert_eq!(
        e.fleet.rovers[1]
            .hardware
            .as_ref()
            .unwrap()
            .token
            .as_deref(),
        Some("unrelated")
    );
    e.fleet.rovers[1].hardware.as_mut().unwrap().token = op.goal_token.clone();
    e.tick(0.3);
    assert_eq!(e.operations[&op.id].phase, "confirmed");
}
#[test]
fn arm_requires_current_control_fresh_telemetry_and_reviewed_authority() {
    let mut e = Engine::new(DemoFleet::default());
    assert!(e.hardware(&who(), "tab", "2", request("arm"), 0.).is_err());
    e.claim(&who(), "tab", 0.).unwrap();
    let mut viewer = who();
    viewer.role = "viewer".into();
    assert!(e.hardware(&viewer, "tab", "2", request("arm"), 0.).is_err());
    for condition in 0..5 {
        e.fleet = DemoFleet::default();
        match condition {
            0 => e.fleet.rovers[1].authority.as_mut().unwrap().safety = "emergency_stop".into(),
            1 => e.fleet.rovers[1].authority.as_mut().unwrap().revision = "2".into(),
            2 => e.fleet.rovers[1].pose_age = Some(0.5),
            3 => e.fleet.rovers[1].hardware.as_mut().unwrap().ready = false,
            _ => e.fleet.rovers[1].authority.as_mut().unwrap().age = 0.5,
        }
        assert!(e.hardware(&who(), "tab", "2", request("arm"), 0.).is_err());
        assert!(!e.fleet.rovers[1].hardware.as_ref().unwrap().armed);
    }
}
#[test]
fn stale_rover_can_be_disarmed_but_never_retried_on_timeout() {
    let mut e = Engine::new(DemoFleet::default());
    e.claim(&who(), "tab", 0.).unwrap();
    e.fleet.rovers[0].pose_age = None;
    e.fleet.rovers[0].authority = None;
    let op = e
        .hardware(&who(), "tab", "1", request("disarm"), 0.)
        .unwrap();
    e.fleet.rovers[0].hardware.as_mut().unwrap().token = None;
    e.tick(11.);
    assert_eq!(e.operations[&op.id].phase, "unconfirmed");
    e.fleet.rovers[0].hardware.as_mut().unwrap().armed = true;
    e.tick(12.);
    assert!(e.fleet.rovers[0].hardware.as_ref().unwrap().armed);
}
