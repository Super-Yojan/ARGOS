use argos_core::{
    CONTROL_CLAIM_SECONDS, ControlClaim, HARDWARE_STATUS_STALE_SECONDS, HardwareObservation,
    drive_block,
};
use serde_json::json;

fn relay(ready: bool, reason: &str) -> HardwareObservation {
    HardwareObservation::from_status(&json!({
        "schema_version": 1,
        "control_mode": "streamed_relay",
        "connected": true,
        "ready": ready,
        "output_active": false,
        "fault": null,
        "reason": reason,
        "age": 0.1
    }))
}

#[test]
fn streamed_relay_status_is_copied_and_legacy_arm_json_is_absent() {
    let status = relay(false, "Motor relay is not ready");
    assert!(status.present);
    assert!(status.connected);
    assert!(!status.ready);
    assert!(!status.output_active);
    assert!(status.fault.is_none());
    assert_eq!(status.reason, "Motor relay is not ready");
    assert_eq!(status.age, 0.1);
    assert_eq!(status.status_label(), "Motor relay not ready");

    let legacy = HardwareObservation::from_status(&json!({
        "ready": true,
        "armed": true,
        "arming": false,
        "reason": "Armed",
        "token": "arm-1",
        "age": 0.1
    }));
    assert!(!legacy.present);
    assert_eq!(
        drive_block(&legacy),
        Some("Hardware status unavailable · connect the updated Terra app")
    );
}

#[test]
fn drive_blocks_follow_connection_fault_readiness_and_age() {
    assert_eq!(
        drive_block(&HardwareObservation::default()),
        Some("Hardware status unavailable · connect the updated Terra app")
    );

    let disconnected = HardwareObservation::from_status(&json!({
        "schema_version": 1,
        "control_mode": "streamed_relay",
        "connected": false,
        "ready": false,
        "output_active": false,
        "reason": "disconnected",
        "age": 0.1
    }));
    assert_eq!(
        drive_block(&disconnected),
        Some("Phone is disconnected from the rover")
    );

    let fault = HardwareObservation::from_status(&json!({
        "schema_version": 1,
        "control_mode": "streamed_relay",
        "connected": true,
        "ready": false,
        "output_active": false,
        "fault": "hardware_gate_open",
        "reason": "fault",
        "age": 0.1
    }));
    assert_eq!(drive_block(&fault), Some("hardware_gate_open"));
    assert_eq!(fault.status_label(), "Motor hardware fault");

    let unnamed = HardwareObservation::from_status(&json!({
        "schema_version": 1,
        "control_mode": "streamed_relay",
        "connected": true,
        "ready": true,
        "output_active": false,
        "fault": 1,
        "reason": "",
        "age": 0.1
    }));
    assert_eq!(unnamed.fault.as_deref(), Some("Motor hardware fault"));

    let mut not_ready = relay(false, "");
    assert_eq!(drive_block(&not_ready), Some("Motor relay is not ready"));
    not_ready.reason = "Bluetooth write stalled".into();
    assert_eq!(drive_block(&not_ready), Some("Bluetooth write stalled"));

    let mut stale = relay(true, "ready");
    stale.age = HARDWARE_STATUS_STALE_SECONDS;
    assert_eq!(
        drive_block(&stale),
        Some("Hardware status is stale · waiting for an update")
    );
    assert_eq!(stale.status_label(), "Hardware status stale");

    let ready = relay(true, "ready");
    assert_eq!(drive_block(&ready), None);
    assert!(ready.ready_and_neutral());
    assert_eq!(ready.status_label(), "Ready · output neutral");

    let mut active = ready.clone();
    active.output_active = true;
    assert_eq!(drive_block(&active), None);
    assert!(!active.ready_and_neutral());
    assert_eq!(active.status_label(), "Ready · output active");
}

#[test]
fn the_control_claim_expires_and_release_drops_it() {
    let mut claim = ControlClaim::new();
    assert!(!claim.held(0.));
    claim.take(10.);
    assert!(claim.held(10.));
    assert!(claim.held(10. + CONTROL_CLAIM_SECONDS - 0.1));
    assert!(!claim.held(10. + CONTROL_CLAIM_SECONDS));
    claim.take(40.);
    claim.renew(41.);
    assert!(claim.held(70.));
    claim.release();
    assert!(!claim.held(41.));
    claim.renew(41.);
    assert!(!claim.held(41.));
}
