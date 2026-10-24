//! Light shaping for teleop twists the dashboard publishes.
//!
//! Defense in depth; Terra enforces the real limits. This does not steer to a
//! heading, integrate yaw over time, or choose a turn duration. `dt` only caps
//! how fast the commanded yaw rate may change. In-place spin is off unless the
//! operator turns it on: a turn with no forward speed publishes zero yaw rate.

use crate::{Result, invalid, valid_token};
use serde_json::{Value, json};

/// Forward speed used when the dashboard has not been given another limit.
pub const DEFAULT_DRIVE_LINEAR: f64 = 0.2;
/// Yaw-rate cap, radians per second. Matches Terra #53's provisional default.
pub const DEFAULT_DRIVE_YAW_RATE: f64 = 0.5;
/// How fast the commanded yaw rate may rise, radians per second squared.
pub const DEFAULT_DRIVE_YAW_ACCEL: f64 = 0.5;
/// How fast the commanded yaw rate may fall, radians per second squared.
pub const DEFAULT_DRIVE_YAW_DECEL: f64 = 0.8;
/// Minimum turn radius while rolling forward or back. In-place spin ignores it.
pub const DEFAULT_MIN_TURN_RADIUS: f64 = 0.5;
/// Measured yaw rate at or above this marks the pose degraded.
pub const DEFAULT_DEGRADED_YAW_RATE: f64 = 0.4;
/// How long the degraded warning stays after the measured rate falls.
/// This holds the operator warning only. It does not command a turn.
pub const DEFAULT_DEGRADED_SETTLE: f64 = 1.0;
/// Key-hold lease. The dashboard refreshes it while a key is down.
pub const DRIVE_DEADMAN_SECONDS: f64 = 0.25;
const SPIN_LINEAR: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TeleopLimits {
    pub linear: f64,
    pub yaw_rate: f64,
    pub yaw_accel: f64,
    pub yaw_decel: f64,
    pub min_turn_radius: f64,
    pub allow_spin: bool,
}
impl Default for TeleopLimits {
    fn default() -> Self {
        Self {
            linear: DEFAULT_DRIVE_LINEAR,
            yaw_rate: DEFAULT_DRIVE_YAW_RATE,
            yaw_accel: DEFAULT_DRIVE_YAW_ACCEL,
            yaw_decel: DEFAULT_DRIVE_YAW_DECEL,
            min_turn_radius: DEFAULT_MIN_TURN_RADIUS,
            allow_spin: false,
        }
    }
}
impl TeleopLimits {
    pub fn validate(self) -> Result<Self> {
        if !self.linear.is_finite()
            || !self.yaw_rate.is_finite()
            || !self.yaw_accel.is_finite()
            || !self.yaw_decel.is_finite()
            || !self.min_turn_radius.is_finite()
            || self.linear < 0.
            || self.linear > 0.5
            || self.yaw_rate <= 0.
            || self.yaw_rate > 1.
            || self.yaw_accel <= 0.
            || self.yaw_accel > 2.
            || self.yaw_decel <= 0.
            || self.yaw_decel > 2.
            || self.min_turn_radius <= 0.
            || self.min_turn_radius > 5.
        {
            return Err(invalid(
                "drive limits must stay within 0.5 m/s, 1 rad/s, 2 rad/s², and 5 m radius",
            ));
        }
        Ok(self)
    }
}

/// Previous commanded yaw rate for the slew limit.
#[derive(Clone, Debug)]
pub struct TeleopGuard {
    pub limits: TeleopLimits,
    angular: f64,
}
impl TeleopGuard {
    pub fn new(limits: TeleopLimits) -> Self {
        Self {
            limits,
            angular: 0.,
        }
    }
    pub fn reset(&mut self) {
        self.angular = 0.;
    }
    pub fn angular(&self) -> f64 {
        self.angular
    }
    /// `unit_linear` and `unit_angular` are held-key directions in `-1..=1`.
    pub fn step(&mut self, unit_linear: f64, unit_angular: f64, dt: f64) -> (f64, f64) {
        let linear = unit_linear.clamp(-1., 1.) * self.limits.linear;
        let mut requested = unit_angular.clamp(-1., 1.) * self.limits.yaw_rate;
        if !self.limits.allow_spin && linear.abs() < SPIN_LINEAR {
            requested = 0.;
        } else if !self.limits.allow_spin {
            // Same radius cap in reverse. |v| / r, never a timed pivot.
            let radius_cap = (linear.abs() / self.limits.min_turn_radius).min(self.limits.yaw_rate);
            requested = requested.clamp(-radius_cap, radius_cap);
        }
        requested = requested.clamp(-self.limits.yaw_rate, self.limits.yaw_rate);
        let dt = if dt.is_finite() && dt > 0. {
            dt.min(0.5)
        } else {
            0.
        };
        let increasing = if self.angular == 0. {
            requested != 0.
        } else {
            requested.abs() > self.angular.abs() + 1e-9
                && requested.signum() == self.angular.signum()
        };
        let slew = if increasing {
            self.limits.yaw_accel
        } else {
            self.limits.yaw_decel
        };
        let max_delta = slew * dt;
        self.angular = (self.angular + (requested - self.angular).clamp(-max_delta, max_delta))
            .clamp(-self.limits.yaw_rate, self.limits.yaw_rate);
        if requested == 0. && self.angular.abs() < 1e-4 {
            self.angular = 0.;
        }
        (linear, self.angular)
    }
}

/// Lease refreshed while a drive key is held. Expiry means the keys are up.
#[derive(Clone, Debug, Default)]
pub struct DriveLease {
    deadline: Option<f64>,
}
impl DriveLease {
    pub fn hold(&mut self, now: f64) {
        if now.is_finite() {
            self.deadline = Some(now + DRIVE_DEADMAN_SECONDS);
        }
    }
    pub fn release(&mut self) {
        self.deadline = None;
    }
    pub fn alive(&self, now: f64) -> bool {
        self.deadline
            .is_some_and(|deadline| now.is_finite() && now < deadline)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DegradeLimits {
    pub yaw_rate: f64,
    pub settle: f64,
}
impl Default for DegradeLimits {
    fn default() -> Self {
        Self {
            yaw_rate: DEFAULT_DEGRADED_YAW_RATE,
            settle: DEFAULT_DEGRADED_SETTLE,
        }
    }
}

/// Operator warning from measured yaw and an optional tracking flag.
///
/// The settle interval only keeps the warning up. It never commands motion.
#[derive(Clone, Debug, Default)]
pub struct PoseWatch {
    sequence: Option<u64>,
    yaw: f64,
    time: f64,
    settle_until: f64,
}
impl PoseWatch {
    pub fn observe(
        &mut self,
        yaw: f64,
        sequence: u64,
        time: f64,
        tracked: Option<bool>,
        limits: DegradeLimits,
    ) -> (bool, &'static str) {
        if tracked == Some(false) {
            return (true, "VIO tracking lost");
        }
        if !yaw.is_finite() || !time.is_finite() {
            return (true, "pose is not finite");
        }
        let mut sharp = false;
        if self.sequence.is_some_and(|seen| seen != sequence) {
            let dt = time - self.time;
            if dt > 0.02 && dt < 1. {
                let rate = wrap_angle(yaw - self.yaw) / dt;
                if rate.is_finite() && rate.abs() >= limits.yaw_rate {
                    sharp = true;
                    self.settle_until = time + limits.settle.max(0.);
                }
            }
        }
        if self.sequence != Some(sequence) {
            self.sequence = Some(sequence);
            self.yaw = yaw;
            self.time = time;
        }
        if sharp {
            return (true, "yaw rate is high");
        }
        if time < self.settle_until {
            return (true, "settling after a turn");
        }
        (false, "")
    }
}

/// Optional tracking on a pose payload. Absent fields stay `None`.
///
/// Terra's phone pose is `x`, `y`, and `yaw` today. `tracked`, `vio_tracked`,
/// `tracking`, or numeric `quality` are honored when a publisher adds them.
pub fn pose_tracking_flag(value: &Value) -> Option<bool> {
    for key in ["tracked", "vio_tracked"] {
        if let Some(flag) = value.get(key).and_then(Value::as_bool) {
            return Some(flag);
        }
    }
    if let Some(state) = value.get("tracking").and_then(Value::as_str) {
        return Some(matches!(state, "ok" | "tracked" | "good" | "ready"));
    }
    if let Some(quality) = value.get("quality").and_then(Value::as_f64)
        && quality.is_finite()
    {
        return Some(quality >= 0.5);
    }
    if let Some(quality) = value.get("quality").and_then(Value::as_str) {
        return Some(!matches!(quality, "lost" | "degraded" | "poor"));
    }
    None
}

/// Label for the yaw-rate heuristic used only when no pose/quality has arrived.
pub fn fallback_degrade_reason(reason: &str) -> String {
    format!("yaw-rate fallback (no pose/quality): {reason}")
}

/// Waypoint sends stop while the pose is degraded. ARGOS teleop does not.
pub fn goal_blocked(degraded: bool) -> Option<&'static str> {
    degraded.then_some("pose is degraded; not sending a goal")
}

/// Fleet teleop twist. Terra treats this publisher as `CommandOrigin::FleetTeleop`
/// and always applies the turning policy. The payload has no exemption field.
pub fn encode_teleop(
    linear: f64,
    angular: f64,
    session: &str,
    sequence: u64,
    run_id: Option<&str>,
    revision: Option<u64>,
) -> Result<Vec<u8>> {
    if !linear.is_finite()
        || !angular.is_finite()
        || linear.abs() > 0.5
        || angular.abs() > 1.
        || !valid_token(session)
    {
        return Err(invalid("invalid teleop command"));
    }
    serde_json::to_vec(&json!({
        "linear": linear,
        "angular": angular,
        "operator_session_id": session,
        "sequence": sequence,
        "run_id": run_id,
        "authority_revision": revision,
    }))
    .map_err(|error| crate::Error(error.to_string()))
}

/// CrabTrapBot body length and width in metres, from its CAD bounds
/// (Zorvane `crab_trap_bot_sim.glb`: 0.601 m long, 0.607 m wide), rounded up.
pub const CRAB_TRAP_BOT_LENGTH: f64 = 0.61;
pub const CRAB_TRAP_BOT_WIDTH: f64 = 0.61;

/// Planner settings ARGOS changes on Terra. Everything else is copied from
/// the rover's published `navigation.parameters`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavigationEdit {
    /// Terra `turning.allow_in_place`: let autonomy spin in place instead of
    /// planning a multi-point turn.
    pub allow_in_place: bool,
    /// Body length and width, metres. Terra validates 0.1..=3.
    pub length: f64,
    pub width: f64,
}
impl NavigationEdit {
    /// CrabTrapBot's body with the given in-place turning choice.
    pub fn crab_trap_bot(allow_in_place: bool) -> Self {
        Self {
            allow_in_place,
            length: CRAB_TRAP_BOT_LENGTH,
            width: CRAB_TRAP_BOT_WIDTH,
        }
    }
}

/// In-place turning as Terra last reported it, if its navigation status says.
pub fn navigation_allows_in_place(navigation: &Value) -> Option<bool> {
    navigation["parameters"]["turning"]["allow_in_place"].as_bool()
}

/// Terra `navigation/settings` request. Terra replaces all parameters at once,
/// so this starts from the rover's published `navigation` status and changes
/// only the fields in `edit`.
pub fn encode_navigation_settings(
    navigation: &Value,
    edit: NavigationEdit,
    token: &str,
    run_id: &str,
    authority_revision: u64,
) -> Result<Vec<u8>> {
    let Some(settings_revision) = navigation["revision"].as_u64() else {
        return Err(invalid("rover has not reported navigation settings"));
    };
    if navigation["schema_version"] != 1
        || !navigation["parameters"].is_object()
        || !navigation["parameters"]["turning"].is_object()
    {
        return Err(invalid("rover has not reported navigation settings"));
    }
    if ![edit.length, edit.width]
        .iter()
        .all(|v| v.is_finite() && (0.1..=3.).contains(v))
    {
        return Err(invalid("body length and width must be 0.1 to 3 m"));
    }
    if !valid_token(token) || run_id.is_empty() {
        return Err(invalid("invalid navigation settings request"));
    }
    let mut parameters = navigation["parameters"].clone();
    parameters["turning"]["allow_in_place"] = json!(edit.allow_in_place);
    parameters["length"] = json!(edit.length);
    parameters["width"] = json!(edit.width);
    serde_json::to_vec(&json!({
        "token": token,
        "run_id": run_id,
        "authority_revision": authority_revision,
        "settings_revision": settings_revision,
        "parameters": parameters,
    }))
    .map_err(|error| crate::Error(error.to_string()))
}

fn wrap_angle(angle: f64) -> f64 {
    (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}
