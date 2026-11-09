//! What the rover says about its safety stop and dump gate, and what to tell
//! the operator when it refuses a Reset stop or a gate test.
//!
//! All of it comes from `autonomy/status`: the tilt guard under
//! `runtime.tilt`, the gate under `gate`, and the receipt for the last
//! request (`token`, `result`, `request_reason`).
use serde_json::Value;

/// Terra's tilt guard (`autonomy/status.runtime.tilt`). While `latched`,
/// the rover holds an emergency stop with reason `tilt_stop`, and refuses
/// Reset stop until it is back within `limit_deg`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TiltReport {
    pub tilt_deg: f64,
    pub limit_deg: f64,
    pub latched: bool,
}

impl TiltReport {
    pub fn decode(status: &Value) -> Option<Self> {
        let tilt = &status["runtime"]["tilt"];
        Some(Self {
            tilt_deg: tilt["tilt_deg"].as_f64()?,
            limit_deg: tilt["limit_deg"].as_f64()?,
            latched: tilt["latched"].as_bool().unwrap_or(false),
        })
    }
    /// Still past the limit, so a Reset stop will be refused.
    pub fn still_tilted(&self) -> bool {
        self.tilt_deg > self.limit_deg
    }
}

/// The dump gate (`autonomy/status.gate`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GateReport {
    /// A gate servo is set (rover profile or TerraPhone Rover setup).
    pub configured: bool,
    /// The gate is commanded open, by a dock or a test.
    pub open: bool,
    /// A gate test is holding it open.
    pub test: bool,
}

impl GateReport {
    /// `None` from a Terra build that does not report the gate.
    pub fn decode(status: &Value) -> Option<Self> {
        let gate = &status["gate"];
        Some(Self {
            configured: gate["configured"].as_bool()?,
            open: gate["open"].as_bool().unwrap_or(false),
            test: gate["test"].as_bool().unwrap_or(false),
        })
    }
    pub fn label(&self) -> &'static str {
        match (self.configured, self.open, self.test) {
            (false, _, _) => "No gate servo set",
            (true, true, true) => "Open (test)",
            (true, true, false) => "Open (dock)",
            (true, false, _) => "Closed",
        }
    }
}

/// The rover's answer to one request, matched by its token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestReceipt {
    pub token: String,
    pub accepted: bool,
    /// Terra's `request_reason`, such as `unhealthy_reset`.
    pub reason: Option<String>,
}

impl RequestReceipt {
    pub fn decode(status: &Value) -> Option<Self> {
        let accepted = match status["result"].as_str()? {
            "accepted" => true,
            "rejected" => false,
            _ => return None,
        };
        Some(Self {
            token: status["token"].as_str()?.to_string(),
            accepted,
            reason: status["request_reason"].as_str().map(str::to_string),
        })
    }
}

/// What the operator does to leave a stop. With the tilt guard still past
/// its limit, Reset stop cannot work yet, so say that instead.
pub fn stop_recovery(tilt: Option<&TiltReport>) -> String {
    match tilt.filter(|tilt| tilt.latched && tilt.still_tilted()) {
        Some(tilt) => format!(
            "Rover tilted {:.0}° (limit {:.0}°) · set it upright, then Reset stop",
            tilt.tilt_deg, tilt.limit_deg
        ),
        None => "Press Reset stop to resume".into(),
    }
}

/// Why the rover refused a Reset stop. Terra refuses with `unhealthy_reset`
/// while the tilt guard is past its limit, or outside L0 while its pose
/// estimate is not ready.
pub fn reset_refusal(reason: Option<&str>, tilt: Option<&TiltReport>) -> String {
    match reason {
        Some("unhealthy_reset") => match tilt.filter(|tilt| tilt.latched && tilt.still_tilted()) {
            Some(tilt) => format!(
                "Reset refused: rover tilted {:.0}° (limit {:.0}°) · set it upright, then Reset stop",
                tilt.tilt_deg, tilt.limit_deg
            ),
            None => "Reset refused: rover pose is not ready · select L0 or wait for tracking, then Reset stop".into(),
        },
        Some("current_authority_required") => {
            "Reset refused: rover state changed while sending · press Reset stop again".into()
        }
        Some(reason) => format!("Reset refused: {}", reason.replace('_', " ")),
        None => "Reset refused by the rover".into(),
    }
}

/// Why the rover refused a gate test.
pub fn gate_refusal(reason: Option<&str>) -> String {
    match reason {
        Some("gate_not_configured") => {
            "Gate refused: no gate servo set · pick one in TerraPhone Rover setup → Dump gate".into()
        }
        Some("dock_running") => "Gate refused: a dock is running and controls the gate".into(),
        Some("emergency_stop") => "Gate refused: Stop is active · Reset stop first".into(),
        Some(reason) => format!("Gate refused: {}", reason.replace('_', " ")),
        None => "Gate refused by the rover".into(),
    }
}
