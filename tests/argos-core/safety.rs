use argos_core::{
    GateReport, RequestReceipt, TiltReport, gate_refusal, reset_refusal, stop_recovery,
};
use serde_json::json;

/// `autonomy/status` as Terra sent it while tipped on its side (session 47dd797c).
fn tilted() -> serde_json::Value {
    json!({
        "safety": "emergency_stop",
        "reason": "tilt_stop",
        "result": "rejected",
        "request_reason": "unhealthy_reset",
        "token": "d47de6fb-e865-494d-a6d4-007c7d732771",
        "runtime": {"tilt": {
            "absolute_tilt_deg": 90.2, "debounce_s": 0.25, "flipped_deg": 120.0,
            "latched": true, "limit_deg": 30.0, "peak_deg": 90.2,
            "reference": "nominal_mount", "tilt_deg": 90.2
        }}
    })
}

#[test]
fn decodes_tilt_and_receipt() {
    let status = tilted();
    let tilt = TiltReport::decode(&status).unwrap();
    assert!(tilt.latched && tilt.still_tilted());
    let receipt = RequestReceipt::decode(&status).unwrap();
    assert_eq!(receipt.token, "d47de6fb-e865-494d-a6d4-007c7d732771");
    assert!(!receipt.accepted);
    assert_eq!(receipt.reason.as_deref(), Some("unhealthy_reset"));
    assert!(TiltReport::decode(&json!({})).is_none());
    assert!(RequestReceipt::decode(&json!({"token": "a", "result": "pending"})).is_none());
}

#[test]
fn reset_refusal_names_the_tilt() {
    let tilt = TiltReport::decode(&tilted()).unwrap();
    assert_eq!(
        reset_refusal(Some("unhealthy_reset"), Some(&tilt)),
        "Reset refused: rover tilted 90° (limit 30°) · set it upright, then Reset stop"
    );
    assert_eq!(
        stop_recovery(Some(&tilt)),
        "Rover tilted 90° (limit 30°) · set it upright, then Reset stop"
    );
    let upright = TiltReport {
        tilt_deg: 3.,
        ..tilt
    };
    assert!(reset_refusal(Some("unhealthy_reset"), Some(&upright)).contains("pose is not ready"));
    assert_eq!(stop_recovery(Some(&upright)), "Press Reset stop to resume");
    assert_eq!(stop_recovery(None), "Press Reset stop to resume");
    assert!(reset_refusal(Some("current_authority_required"), None).contains("again"));
}

#[test]
fn gate_report_and_refusals() {
    assert!(GateReport::decode(&json!({})).is_none());
    let gate =
        GateReport::decode(&json!({"gate": {"configured": true, "open": true, "test": true}}))
            .unwrap();
    assert_eq!(gate.label(), "Open (test)");
    let unset = GateReport::decode(&json!({"gate": {"configured": false}})).unwrap();
    assert_eq!(unset.label(), "No gate servo set");
    assert!(gate_refusal(Some("gate_not_configured")).contains("Dump gate"));
    assert!(gate_refusal(Some("emergency_stop")).contains("Reset stop"));
    assert!(gate_refusal(Some("dock_running")).contains("dock"));
}
