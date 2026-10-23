use argos_core::*;
use serde_json::json;
fn relay() -> serde_json::Value {
    json!({"schema_version":1,"control_mode":"streamed_relay","connected":true,"ready":true,"output_active":false,"fault":null,"age":0.1})
}
#[test]
fn relay_readiness_requires_current_schema_and_fresh_status() {
    let mut value = relay();
    assert!(drive_block(&HardwareObservation::from_status(&value)).is_none());
    value["age"] = HARDWARE_STATUS_STALE_SECONDS.into();
    assert!(drive_block(&HardwareObservation::from_status(&value)).is_some());
    assert!(
        drive_block(&HardwareObservation::from_status(
            &json!({"version":1,"armed":true,"ready":true})
        ))
        .is_some()
    );
}
#[test]
fn active_output_is_drivable_but_cannot_change_configuration() {
    let mut value = relay();
    value["output_active"] = true.into();
    let h = HardwareObservation::from_status(&value);
    assert!(drive_block(&h).is_none());
    assert!(!h.ready_and_neutral());
}
#[test]
fn disconnect_fault_and_unready_block_drive() {
    for (key, value) in [
        ("connected", json!(false)),
        ("ready", json!(false)),
        ("fault", json!("motor_failed")),
    ] {
        let mut h = relay();
        h[key] = value;
        assert!(drive_block(&HardwareObservation::from_status(&h)).is_some());
    }
}
#[test]
fn local_claim_expires_without_renewal() {
    let mut claim = ControlClaim::new();
    claim.take(1.0);
    assert!(claim.held(1.1));
    assert!(!claim.held(31.0));
    claim.take(40.);
    claim.release();
    assert!(!claim.held(40.));
}
