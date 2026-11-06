//! Whether a rover is still sending its data. The router link can stay up
//! while the phone stops publishing, so the dashboard judges each rover by
//! its own pose and hardware status (ARGOS#109).
use crate::{HARDWARE_STATUS_STALE_SECONDS, HardwareObservation, STALE_SECONDS};

/// Pose and hardware status older than this count as not reporting.
pub const ROVER_DATA_STALE_SECONDS: f64 = STALE_SECONDS;
/// A rover silent this long reads as link lost, with a next step.
pub const ROVER_LINK_LOST_SECONDS: f64 = 15.;
/// The dashboard shows a rover as not reporting only after this much silence.
/// TerraPhone on outdoor Wi-Fi often goes quiet for 2-4 s while it keeps
/// driving its goal, and flagging each of those made the rover flicker.
pub const ROVER_QUIET_SECONDS: f64 = 5.;
/// A rover shown as not reporting must send data this long before it reads
/// as fresh again, so a short burst between gaps does not flash it back.
pub const ROVER_RECOVER_SECONDS: f64 = 3.;

/// One rover's data freshness, as the dashboard shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Reporting {
    /// Pose or hardware status is fresh.
    #[default]
    Fresh,
    /// Pose and hardware status are both stale. The rover holds.
    NotReporting { silent_s: f64 },
    /// Silent for [`ROVER_LINK_LOST_SECONDS`] or more.
    LinkLost { silent_s: f64 },
}

impl Reporting {
    /// Judge a rover from its pose age and hardware status. A rover that
    /// never reported hardware status is judged by its pose alone.
    pub fn classify(pose_age: Option<f64>, hardware: &HardwareObservation) -> Self {
        let fresh = |age: f64| age.is_finite() && (0.0..ROVER_DATA_STALE_SECONDS).contains(&age);
        let hardware_age = hardware.present.then_some(hardware.age);
        if pose_age.is_some_and(fresh)
            || hardware_age.is_some_and(|age| {
                age.is_finite() && (0.0..HARDWARE_STATUS_STALE_SECONDS).contains(&age)
            })
        {
            return Self::Fresh;
        }
        // The newest thing the rover sent says how long it has been silent.
        let silent_s = [pose_age, hardware_age]
            .into_iter()
            .flatten()
            .filter(|age| age.is_finite() && *age >= 0.)
            .reduce(f64::min)
            .unwrap_or(f64::INFINITY);
        if silent_s >= ROVER_LINK_LOST_SECONDS {
            Self::LinkLost { silent_s }
        } else {
            Self::NotReporting { silent_s }
        }
    }
    /// Wire name for the session log: `fresh`, `not_reporting` or `link_lost`.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::NotReporting { .. } => "not_reporting",
            Self::LinkLost { .. } => "link_lost",
        }
    }
    pub fn is_fresh(self) -> bool {
        self == Self::Fresh
    }
    /// Seconds since the rover last sent pose or hardware status.
    pub fn silent_s(self) -> Option<f64> {
        match self {
            Self::Fresh => None,
            Self::NotReporting { silent_s } | Self::LinkLost { silent_s } => Some(silent_s),
        }
    }
    /// `Not reporting · holding · 12 s` or `Link lost · 40 s`.
    pub fn label(self) -> Option<String> {
        let timer = |s: f64| {
            if s.is_finite() {
                format!("{:.0} s", s.floor())
            } else {
                "no data yet".into()
            }
        };
        match self {
            Self::Fresh => None,
            Self::NotReporting { silent_s } => Some(format!("Not reporting · {}", timer(silent_s))),
            Self::LinkLost { silent_s } => Some(format!("Link lost · {}", timer(silent_s))),
        }
    }
    /// What the operator should do now.
    pub fn next_step(self) -> Option<&'static str> {
        match self {
            Self::Fresh => None,
            Self::NotReporting { .. } => Some(
                "Waiting for the rover's data. A rover on its own level keeps working on its goal meanwhile. In teleop it holds: release and press the drive keys again.",
            ),
            Self::LinkLost { .. } => Some(
                "Check that TerraPhone is open in the foreground and on Wi-Fi. When it reports again, resend the goal if the rover dropped it, or take over. In teleop, release and press the drive keys again.",
            ),
        }
    }
}

/// Hysteresis over [`Reporting::classify`] for one rover, so short telemetry
/// gaps do not make the rover and its notices flicker.
///
/// - Fresh to not reporting only after [`ROVER_QUIET_SECONDS`] of silence.
/// - Back to fresh only after [`ROVER_RECOVER_SECONDS`] of steady data. Until
///   then the last quiet reading is kept, timer frozen.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReportingHold {
    shown: Reporting,
    fresh_since: Option<f64>,
}

impl ReportingHold {
    /// Feed the raw reading at `now` (seconds, any steady clock); returns
    /// what the dashboard should show.
    pub fn observe(&mut self, raw: Reporting, now: f64) -> Reporting {
        match raw {
            Reporting::Fresh => {
                let since = *self.fresh_since.get_or_insert(now);
                if !self.shown.is_fresh() && now - since >= ROVER_RECOVER_SECONDS {
                    self.shown = Reporting::Fresh;
                }
            }
            quiet => {
                self.fresh_since = None;
                let silent_s = quiet.silent_s().unwrap_or(f64::INFINITY);
                if !self.shown.is_fresh() || silent_s >= ROVER_QUIET_SECONDS {
                    self.shown = quiet;
                }
            }
        }
        self.shown
    }
}
