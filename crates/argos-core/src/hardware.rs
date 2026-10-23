//! Phone-reported motor relay readiness and local operator ownership.
use serde_json::Value;

/// Use the shared manual telemetry window for phone hardware observations.
pub const HARDWARE_STATUS_STALE_SECONDS: f64 = crate::STALE_SECONDS;

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

/// Current `{prefix}/{id}/hardware/status` from the phone.
#[derive(Clone, Debug, PartialEq)]
pub struct HardwareObservation {
    pub present: bool,
    pub connected: bool,
    pub ready: bool,
    pub output_active: bool,
    pub fault: Option<String>,
    pub reason: String,
    pub age: f64,
}
impl Default for HardwareObservation {
    fn default() -> Self {
        Self {
            present: false,
            connected: false,
            ready: false,
            output_active: false,
            fault: None,
            reason: String::new(),
            age: f64::INFINITY,
        }
    }
}
impl HardwareObservation {
    pub fn from_status(value: &Value) -> Self {
        if value["schema_version"] != 1
            || value["control_mode"] != "streamed_relay"
            || !["ready", "connected", "output_active"]
                .iter()
                .all(|key| value[*key].is_boolean())
        {
            return Self::default();
        }
        Self {
            present: true,
            connected: value["connected"] == true,
            ready: value["ready"] == true,
            output_active: value["output_active"] == true,
            fault: value
                .get("fault")
                .filter(|fault| !fault.is_null())
                .map(|fault| fault.as_str().unwrap_or("Motor hardware fault").to_owned()),
            reason: value["reason"].as_str().unwrap_or("").to_owned(),
            age: value["age"].as_f64().unwrap_or(f64::INFINITY),
        }
    }
    pub fn ready_and_neutral(&self) -> bool {
        drive_block(self).is_none() && !self.output_active
    }
    pub fn status_label(&self) -> &'static str {
        if !self.present {
            "Hardware status unavailable"
        } else if !self.connected {
            "Rover disconnected"
        } else if self.fault.is_some() {
            "Motor hardware fault"
        } else if !self.ready {
            "Motor relay not ready"
        } else if !self.age.is_finite() || !(0.0..HARDWARE_STATUS_STALE_SECONDS).contains(&self.age)
        {
            "Hardware status stale"
        } else if self.output_active {
            "Ready · output active"
        } else {
            "Ready · output neutral"
        }
    }
}
/// Neutral output is a normal state and never prevents starting a command stream.
pub fn drive_block(status: &HardwareObservation) -> Option<&str> {
    if !status.present {
        return Some("Hardware status unavailable · connect the updated Terra app");
    }
    if !status.connected {
        return Some("Phone is disconnected from the rover");
    }
    if let Some(fault) = status.fault.as_deref() {
        return Some(fault);
    }
    if !status.ready {
        return Some(if status.reason.is_empty() {
            "Motor relay is not ready"
        } else {
            &status.reason
        });
    }
    if !status.age.is_finite() || !(0.0..HARDWARE_STATUS_STALE_SECONDS).contains(&status.age) {
        return Some("Hardware status is stale · waiting for an update");
    }
    None
}
