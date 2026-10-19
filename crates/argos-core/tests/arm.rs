use argos_core::{
    ARM_ACK_TIMEOUT, ARM_ACK_TIMEOUT_SECONDS, ARM_HOLD_SECONDS, ARM_TO_DRIVE, ArmControl, ArmGate,
    ControlClaim, HardwareCommand, HardwareObservation, arm_disabled_reason, drive_block,
    encode_hardware,
};
use serde_json::json;

fn open<'a>() -> ArmGate<'a> {
    ArmGate {
        selected: true,
        link_healthy: true,
        pose_fresh: true,
        local_latch: false,
        safety: "clear",
        claim_held: true,
        run_ready: true,
        hardware_present: true,
    }
}

fn disarmed() -> HardwareObservation {
    HardwareObservation {
        present: true,
        armed: false,
        arming: false,
        reason: "disarmed".into(),
        token: String::new(),
        result: String::new(),
        age: 0.1,
    }
}

fn hold_until_arm(control: &mut ArmControl) -> HardwareCommand {
    assert!(control.press(open()).is_none());
    let command = control
        .tick(ARM_HOLD_SECONDS, open(), &disarmed(), "run", 4)
        .expect("hold publishes arm");
    assert!(command.is_arm());
    command
}

#[test]
fn a_click_does_not_arm_and_a_one_second_hold_does() {
    let mut control = ArmControl::new();
    assert!(control.press(open()).is_none());
    assert!(control.tick(0.2, open(), &disarmed(), "run", 4).is_none());
    control.release();
    assert!(control.tick(5., open(), &disarmed(), "run", 4).is_none());
    assert!(control.holding().is_none());
    assert_eq!(control.status_label(&disarmed()), "Disarmed");

    let command = hold_until_arm(&mut control);
    let HardwareCommand::Arm {
        token,
        run_id,
        authority_revision,
    } = command
    else {
        panic!("expected arm");
    };
    assert_eq!(token, "arm-1");
    assert_eq!(run_id, "run");
    assert_eq!(authority_revision, 4);
    assert_eq!(control.status_label(&disarmed()), "arming…");
    let bytes = encode_hardware(&HardwareCommand::Arm {
        token,
        run_id,
        authority_revision,
    })
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        json!({
            "action": "arm",
            "token": "arm-1",
            "run_id": "run",
            "authority_revision": 4
        })
    );
}

#[test]
fn arm_stays_disabled_for_each_blocked_gate() {
    let cases = [
        (
            ArmGate {
                selected: false,
                ..open()
            },
            "Select this rover before arming",
        ),
        (
            ArmGate {
                link_healthy: false,
                ..open()
            },
            "Link is down",
        ),
        (
            ArmGate {
                pose_fresh: false,
                ..open()
            },
            "Pose is stale",
        ),
        (
            ArmGate {
                local_latch: true,
                ..open()
            },
            "Safety stop is latched",
        ),
        (
            ArmGate {
                safety: "emergency_stop",
                ..open()
            },
            "Safety stop is latched",
        ),
        (
            ArmGate {
                safety: "hold",
                ..open()
            },
            "Authority safety is not clear",
        ),
        (
            ArmGate {
                claim_held: false,
                ..open()
            },
            "Take control to arm this vehicle.",
        ),
        (
            ArmGate {
                run_ready: false,
                ..open()
            },
            "Vehicle authority unavailable",
        ),
        (
            ArmGate {
                hardware_present: false,
                ..open()
            },
            "Hardware status unavailable",
        ),
    ];
    for (gate, reason) in cases {
        assert_eq!(arm_disabled_reason(gate), Some(reason));
        let mut control = ArmControl::new();
        assert_eq!(control.press(gate), Some(reason));
        assert!(control.holding().is_none());
        assert!(
            control
                .tick(ARM_HOLD_SECONDS + 1., gate, &disarmed(), "run", 4)
                .is_none()
        );
    }
}

#[test]
fn disarm_is_one_click_even_when_arm_is_blocked() {
    let mut control = ArmControl::new();
    let blocked = ArmGate {
        selected: false,
        link_healthy: false,
        pose_fresh: false,
        local_latch: true,
        safety: "emergency_stop",
        claim_held: false,
        run_ready: false,
        hardware_present: false,
    };
    assert!(control.press(blocked).is_some());
    let command = control.disarm("");
    let HardwareCommand::Disarm { token, run_id } = command.clone() else {
        panic!("expected disarm");
    };
    assert_eq!(token, "disarm-1");
    assert!(run_id.is_empty());
    assert!(!command.is_arm());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&encode_hardware(&command).unwrap()).unwrap(),
        json!({
            "action": "disarm",
            "token": "disarm-1",
            "run_id": "",
            "authority_revision": 0
        })
    );
}

#[test]
fn reset_cancels_a_hold_and_does_not_arm() {
    let mut control = ArmControl::new();
    assert!(control.press(open()).is_none());
    assert!(control.tick(0.4, open(), &disarmed(), "run", 4).is_none());
    let command = control.reset();
    assert!(matches!(command, HardwareCommand::Reset));
    assert!(!command.is_arm());
    assert!(encode_hardware(&command).is_err());
    assert!(
        control
            .tick(ARM_HOLD_SECONDS, open(), &disarmed(), "run", 4)
            .is_none()
    );
    assert_eq!(control.status_label(&disarmed()), "Disarmed");
    assert!(control.selection().is_none());
}

#[test]
fn acknowledgement_comes_from_status_and_times_out() {
    let mut control = ArmControl::new();
    hold_until_arm(&mut control);
    let mut waiting = disarmed();
    waiting.token = "someone-else".into();
    waiting.armed = true;
    assert!(control.tick(1., open(), &waiting, "run", 4).is_none());
    assert_eq!(control.status_label(&waiting), "arming…");

    let mut rejected = disarmed();
    rejected.token = "arm-1".into();
    rejected.result = "rejected: L3 lacks wheel feedback".into();
    rejected.reason = "L3 lacks wheel feedback".into();
    assert!(control.tick(0.1, open(), &rejected, "run", 4).is_none());
    assert_eq!(
        control.error.as_deref(),
        Some("rejected: L3 lacks wheel feedback")
    );
    assert_eq!(control.status_label(&rejected), "Disarmed");
    assert!(!control.pending_arm());

    hold_until_arm(&mut control);
    let mut silent = disarmed();
    silent.reason = "L3 lacks wheel feedback".into();
    assert!(
        control
            .tick(ARM_ACK_TIMEOUT_SECONDS - 0.1, open(), &silent, "run", 4)
            .is_none()
    );
    assert_eq!(control.status_label(&silent), "arming…");
    assert!(control.tick(0.1, open(), &silent, "run", 4).is_none());
    assert_eq!(
        control.error.as_deref(),
        Some(&format!("{ARM_ACK_TIMEOUT} · L3 lacks wheel feedback")[..])
    );
    assert_eq!(control.status_label(&disarmed()), "Disarmed");

    hold_until_arm(&mut control);
    let mut armed = disarmed();
    armed.token = "arm-3".into();
    armed.armed = true;
    armed.result = "accepted".into();
    armed.reason = "armed".into();
    assert!(control.tick(0.1, open(), &armed, "run", 4).is_none());
    assert!(control.error.is_none());
    assert_eq!(control.status_label(&armed), "Armed");
    assert!(drive_block(&disarmed()) == Some(ARM_TO_DRIVE));
    assert!(drive_block(&armed).is_none());
}

#[test]
fn the_control_claim_expires_and_release_drops_it() {
    let mut claim = ControlClaim::new();
    assert!(!claim.held(0.));
    claim.take(10.);
    assert!(claim.held(10.));
    assert!(claim.held(39.9));
    assert!(!claim.held(40.));
    claim.take(40.);
    claim.renew(41.);
    assert!(claim.held(70.));
    claim.release();
    assert!(!claim.held(41.));
}

#[test]
fn a_hold_that_loses_its_gate_does_not_publish() {
    let mut control = ArmControl::new();
    assert!(control.press(open()).is_none());
    let blocked = ArmGate {
        claim_held: false,
        ..open()
    };
    assert!(
        control
            .tick(ARM_HOLD_SECONDS, blocked, &disarmed(), "run", 4)
            .is_none()
    );
    assert_eq!(
        control.error.as_deref(),
        Some("Take control to arm this vehicle.")
    );
    assert!(!control.pending_arm());
}
