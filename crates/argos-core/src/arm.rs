//! Hold-to-arm for `{prefix}/{id}/hardware`.
//!
//! The payload matches the command Terra admits and the operator client
//! publishes: `{"action":"arm"|"disarm","token","run_id","authority_revision"}`.
//! Disarm uses authority revision 0. A click never reaches that publish.
//! Armed and disarmed come from `{prefix}/{id}/hardware/status`. Reset is the
//! existing `{prefix}/{id}/safety` action `reset` and does not arm.

use crate::{Result, invalid, valid_token};
use serde_json::{Value, json};

/// How long the operator holds before an arm command is published.
pub const ARM_HOLD_SECONDS: f64 = 1.0;
/// Window before an arm or disarm request is considered unconfirmed.
pub const ARM_ACK_TIMEOUT_SECONDS: f64 = 10.0;
pub const ARM_ACK_TIMEOUT: &str =
    "Hardware outcome unconfirmed; not retried. Check live hardware status.";
pub const ARM_TO_DRIVE: &str = "Arm to drive";
/// Local operator lease. Matches the service tab claim. It is not a Zenoh key.
pub const CONTROL_CLAIM_SECONDS: f64 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlClaim {
    until: Option<u64>,
}

impl ControlClaim {
    pub fn new() -> Self {
        Self { until: None }
    }

    pub fn take(&mut self, now: f64) {
        self.until = Some(deadline(now));
    }

    pub fn renew(&mut self, now: f64) {
        if self.held(now) {
            self.take(now);
        }
    }

    pub fn release(&mut self) {
        self.until = None;
    }

    pub fn held(&self, now: f64) -> bool {
        self.until
            .is_some_and(|until| now.is_finite() && (now * 1000.).floor() < until as f64)
    }
}

fn deadline(now: f64) -> u64 {
    let now_ms = if now.is_finite() {
        (now * 1000.).floor()
    } else {
        0.
    };
    (now_ms + CONTROL_CLAIM_SECONDS * 1000.).max(0.) as u64
}

impl Default for ControlClaim {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ArmGate<'a> {
    pub selected: bool,
    pub link_healthy: bool,
    pub pose_fresh: bool,
    pub local_latch: bool,
    pub safety: &'a str,
    pub claim_held: bool,
    pub run_ready: bool,
    pub hardware_present: bool,
}

/// Why Arm stays disabled. `None` means a hold may start.
pub fn arm_disabled_reason(gate: ArmGate<'_>) -> Option<&'static str> {
    if !gate.selected {
        return Some("Select this rover before arming");
    }
    if !gate.link_healthy {
        return Some("Link is down");
    }
    if !gate.pose_fresh {
        return Some("Pose is stale");
    }
    if gate.local_latch || gate.safety == "emergency_stop" {
        return Some("Safety stop is latched");
    }
    if gate.safety != "clear" {
        return Some("Authority safety is not clear");
    }
    if !gate.claim_held {
        return Some("Take control to arm this vehicle.");
    }
    if !gate.run_ready {
        return Some("Vehicle authority unavailable");
    }
    if !gate.hardware_present {
        return Some("Hardware status unavailable");
    }
    None
}

/// Live `{prefix}/{id}/hardware/status`. Missing status is not disarmed.
#[derive(Clone, Debug, PartialEq)]
pub struct HardwareObservation {
    pub present: bool,
    pub armed: bool,
    pub arming: bool,
    pub reason: String,
    pub stop_detail: String,
    pub token: String,
    pub result: String,
    pub age: f64,
}

impl Default for HardwareObservation {
    fn default() -> Self {
        Self {
            present: false,
            armed: false,
            arming: false,
            reason: String::new(),
            stop_detail: String::new(),
            token: String::new(),
            result: String::new(),
            age: f64::INFINITY,
        }
    }
}

impl HardwareObservation {
    pub fn motion_paused(&self) -> bool {
        self.reason == "watchdog_paused"
    }

    pub fn from_status(value: &Value) -> Self {
        if !value.is_object() {
            return Self::default();
        }
        Self {
            present: true,
            armed: value["armed"] == true,
            arming: value["arming"] == true,
            reason: if value["motion_paused"] == true || value["stop_reason"] == "watchdog_paused" {
                "watchdog_paused".into()
            } else {
                value["reason"].as_str().unwrap_or("").to_string()
            },
            stop_detail: ["stop_diagnostic", "fault", "stop_reason", "reason"]
                .into_iter()
                .filter_map(|key| value[key].as_str())
                .find(|reason| {
                    !reason.trim().is_empty()
                        && !matches!(
                            *reason,
                            "watchdog_paused"
                                | "requested"
                                | "boot"
                                | "connected"
                                | "status_subscribed"
                                | "armed"
                                | "Armed"
                                | "disarmed"
                                | "Disarmed"
                                | "arming"
                                | "Arming at safe output"
                                | "Armed · motion paused · Resume required"
                        )
                })
                .unwrap_or("")
                .to_string(),
            token: value["token"].as_str().unwrap_or("").to_string(),
            result: value["result"].as_str().unwrap_or("").to_string(),
            age: value["age"].as_f64().unwrap_or(f64::INFINITY),
        }
    }
}

/// Preserve the blocking state; unavailable telemetry must never be labelled disarmed.
pub fn drive_block(status: &HardwareObservation) -> Option<&str> {
    if !status.present {
        return Some("Hardware status unavailable");
    }
    if !status.age.is_finite() || !(0.0..0.5).contains(&status.age) {
        return Some("Hardware status is stale · waiting for an update");
    }
    if status.arming {
        return Some("Arming · wait for hardware confirmation");
    }
    if status.motion_paused() || !status.armed {
        if !status.stop_detail.is_empty() {
            return Some(&status.stop_detail);
        }
        if status.motion_paused() {
            return Some("Motion paused · command stream interrupted · Resume required");
        }
        return Some(ARM_TO_DRIVE);
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
pub enum HardwareCommand {
    Arm {
        token: String,
        run_id: String,
        authority_revision: u64,
    },
    Disarm {
        token: String,
        run_id: String,
    },
    /// `{prefix}/{id}/safety` with action `reset`. Not an arm.
    Reset,
}

impl HardwareCommand {
    pub fn is_arm(&self) -> bool {
        matches!(self, Self::Arm { .. })
    }
}

/// JSON body for `{prefix}/{id}/hardware`. Reset is not a hardware command.
pub fn encode_hardware(command: &HardwareCommand) -> Result<Vec<u8>> {
    let (action, token, run_id, authority_revision) = match command {
        HardwareCommand::Arm {
            token,
            run_id,
            authority_revision,
        } => ("arm", token.as_str(), run_id.as_str(), *authority_revision),
        HardwareCommand::Disarm { token, run_id } => ("disarm", token.as_str(), run_id.as_str(), 0),
        HardwareCommand::Reset => return Err(invalid("reset is a safety action")),
    };
    if !matches!(action, "arm" | "disarm") || !valid_token(token) {
        return Err(invalid("invalid hardware request"));
    }
    serde_json::to_vec(&json!({
        "action": action,
        "token": token,
        "run_id": run_id,
        "authority_revision": authority_revision,
    }))
    .map_err(|error| crate::Error(error.to_string()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingKind {
    Arm,
    Disarm,
}

#[derive(Clone, Debug, PartialEq)]
struct Pending {
    kind: PendingKind,
    token: String,
    elapsed: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArmControl {
    holding: Option<f64>,
    pending: Option<Pending>,
    sequence: u64,
    pub error: Option<String>,
}

impl Default for ArmControl {
    fn default() -> Self {
        Self::new()
    }
}

impl ArmControl {
    pub fn new() -> Self {
        Self {
            holding: None,
            pending: None,
            sequence: 0,
            error: None,
        }
    }

    /// Press starts a hold. It does not publish.
    pub fn press(&mut self, gate: ArmGate<'_>) -> Option<&'static str> {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.kind == PendingKind::Arm)
        {
            return Some("arming…");
        }
        if let Some(reason) = arm_disabled_reason(gate) {
            self.holding = None;
            return Some(reason);
        }
        if self.holding.is_none() {
            self.holding = Some(0.);
            self.error = None;
        }
        None
    }

    /// Release before the hold completes. A click ends here and publishes nothing.
    pub fn release(&mut self) {
        self.holding = None;
    }

    pub fn cancel_hold(&mut self) {
        self.holding = None;
    }

    pub fn holding(&self) -> Option<f64> {
        self.holding
    }

    pub fn pending_arm(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.kind == PendingKind::Arm)
    }

    /// Source and level choices do not publish hardware.
    pub fn selection(&self) -> Option<HardwareCommand> {
        None
    }

    /// One click. Allowed while the link, pose, claim, or safety gate would block arming.
    pub fn disarm(&mut self, run_id: &str) -> HardwareCommand {
        self.holding = None;
        self.error = None;
        let token = self.next_token("disarm");
        self.pending = Some(Pending {
            kind: PendingKind::Disarm,
            token: token.clone(),
            elapsed: 0.,
        });
        HardwareCommand::Disarm {
            token,
            run_id: run_id.to_string(),
        }
    }

    /// Clears a latched stop. The returned command is never an arm.
    pub fn reset(&mut self) -> HardwareCommand {
        self.holding = None;
        HardwareCommand::Reset
    }

    pub fn tick(
        &mut self,
        dt: f64,
        gate: ArmGate<'_>,
        status: &HardwareObservation,
        run_id: &str,
        authority_revision: u64,
    ) -> Option<HardwareCommand> {
        let dt = if dt.is_finite() && dt > 0. { dt } else { 0. };
        if let Some(pending) = self.pending.as_mut() {
            pending.elapsed += dt;
        }
        let mut command = None;
        if let Some(elapsed) = self.holding.as_mut() {
            *elapsed += dt;
            if *elapsed >= ARM_HOLD_SECONDS {
                self.holding = None;
                command = self.finish_hold(gate, run_id, authority_revision);
            }
        }
        self.observe(status);
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.elapsed >= ARM_ACK_TIMEOUT_SECONDS)
        {
            let reason = status.reason.trim();
            self.error = Some(if reason.is_empty() {
                ARM_ACK_TIMEOUT.to_string()
            } else {
                format!("{ARM_ACK_TIMEOUT} · {reason}")
            });
            self.pending = None;
        }
        command
    }

    pub fn status_label(&self, status: &HardwareObservation) -> String {
        if self.pending_arm() || (status.present && status.arming && !status.armed) {
            return "arming…".into();
        }
        if status.present && status.armed {
            if status.motion_paused() {
                return "Armed · motion paused".into();
            }
            return "Armed".into();
        }
        if status.present {
            return "Disarmed".into();
        }
        "Hardware status unavailable".into()
    }

    fn finish_hold(
        &mut self,
        gate: ArmGate<'_>,
        run_id: &str,
        authority_revision: u64,
    ) -> Option<HardwareCommand> {
        if let Some(reason) = arm_disabled_reason(gate) {
            self.error = Some(reason.into());
            return None;
        }
        let token = self.next_token("arm");
        self.pending = Some(Pending {
            kind: PendingKind::Arm,
            token: token.clone(),
            elapsed: 0.,
        });
        self.error = None;
        Some(HardwareCommand::Arm {
            token,
            run_id: run_id.to_string(),
            authority_revision,
        })
    }

    fn observe(&mut self, status: &HardwareObservation) {
        let Some(pending) = self.pending.clone() else {
            return;
        };
        let fresh = status.present && status.age.is_finite() && (0.0..0.5).contains(&status.age);
        let token_matches = fresh && status.token == pending.token;
        if token_matches && status.result.starts_with("rejected") {
            self.error = Some(status.result.clone());
            self.pending = None;
            return;
        }
        let confirmed = match pending.kind {
            PendingKind::Arm => {
                token_matches && status.armed && !status.arming && !status.motion_paused()
            }
            PendingKind::Disarm => {
                fresh
                    && !status.armed
                    && !status.arming
                    && (token_matches || status.token.is_empty())
            }
        };
        if confirmed {
            self.error = None;
            self.pending = None;
        }
    }

    fn next_token(&mut self, kind: &str) -> String {
        self.sequence += 1;
        format!("{kind}-{}", self.sequence)
    }
}
