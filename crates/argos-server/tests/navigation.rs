use argos_server::{engine::Engine, fleet::DemoFleet, types::*};
fn user() -> Identity {
    Identity {
        id: "1".into(),
        login: "operator".into(),
        role: "operator".into(),
        csrf: "c".into(),
    }
}
fn request() -> NavigationRequest {
    NavigationRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        run_id: "demo-run".into(),
        revision: "1".into(),
        settings_revision: "0".into(),
        parameters: NavigationParameters {
            length: 0.8,
            width: 0.6,
            clearance: 0.1,
            max_linear: 0.5,
            max_angular: 1.,
            horizon: 2.,
        },
    }
}
#[test]
fn quick_settings_require_disarmed_hardware_and_confirm_exact_applied_values() {
    let mut e = Engine::new(DemoFleet::default());
    e.claim(&user(), "tab", 0.).unwrap();
    assert!(e.navigation(&user(), "tab", "1", request(), 0.).is_err());
    let r = request();
    let op = e.navigation(&user(), "tab", "2", r.clone(), 0.).unwrap();
    e.tick(0.1);
    assert_eq!(e.operations[&op.id].phase, "confirmed");
    assert_eq!(
        e.fleet.rovers[1].navigation.as_ref().unwrap().parameters,
        r.parameters
    );
    assert_eq!(e.navigation(&user(), "tab", "2", r, 0.2).unwrap().id, op.id);
    assert_eq!(e.fleet.rovers[1].navigation.as_ref().unwrap().revision, "1");
}
#[test]
fn dimensions_and_avoidance_limits_reject_invalid_or_outdated_updates() {
    let mut e = Engine::new(DemoFleet::default());
    e.claim(&user(), "tab", 0.).unwrap();
    let mut r = request();
    r.parameters.width = -1.;
    assert!(e.navigation(&user(), "tab", "2", r, 0.).is_err());
    let mut r = request();
    r.settings_revision = "old".into();
    assert!(e.navigation(&user(), "tab", "2", r, 0.).is_err());
    e.fleet.rovers[1].hardware.as_mut().unwrap().age = 0.5;
    assert!(e.navigation(&user(), "tab", "2", request(), 0.).is_err());
}
