//! Whether a rover is still sending its data. The router link can stay up
//! while the phone stops publishing, so the dashboard judges each rover by
//! its own pose and hardware status (ARGOS#109).
use crate::{HARDWARE_STATUS_STALE_SECONDS, HardwareObservation, STALE_SECONDS};

/// Pose and hardware status older than this count as not reporting.
pub const ROVER_DATA_STALE_SECONDS: f64 = STALE_SECONDS;
/// A rover silent this long reads as link lost, with a next step.
pub const ROVER_LINK_LOST_SECONDS: f64 = 15.;

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
            Self::NotReporting { silent_s } => {
                Some(format!("Not reporting · holding · {}", timer(silent_s)))
            }
            Self::LinkLost { silent_s } => Some(format!("Link lost · {}", timer(silent_s))),
        }
    }
    /// What the operator should do now.
    pub fn next_step(self) -> Option<&'static str> {
        match self {
            Self::Fresh => None,
            Self::NotReporting { .. } => Some(
                "Waiting for the rover. It holds still and carries on by itself when its data returns.",
            ),
            Self::LinkLost { .. } => Some(
                "Check that TerraPhone is open in the foreground and on Wi-Fi. The rover drops an L2/L3 goal after 30 s without data, so resend the goal or take over once it reports again.",
            ),
        }
    }
}
