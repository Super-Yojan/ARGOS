use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Identity {
    pub id: String,
    pub login: String,
    pub role: String,
    pub csrf: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct Pose {
    pub x: f64,
    pub y: f64,
    pub yaw: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Authority {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub reason: Option<String>,
    pub run_id: String,
    pub revision: String,
    pub requested_level: String,
    pub supported_levels: Vec<String>,
    pub safety: String,
    pub age: f64,
    pub token: Option<String>,
    pub result: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Hardware {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub arming: Option<bool>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub token: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub result: Option<String>,
    pub ready: bool,
    pub armed: bool,
    pub reason: String,
    pub age: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Reference {
    pub latitude: f64,
    pub longitude: f64,
    pub anchor_x: f64,
    pub anchor_y: f64,
    pub rotation: f64,
    pub frame_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Goal {
    pub state: String,
    pub token: Option<String>,
    pub x: f64,
    pub y: f64,
    pub distance: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct GpsFix {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: f64,
    pub age: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Rover {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub gps: Option<GpsFix>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub navigation: Option<NavigationState>,
    pub id: String,
    pub membership: String,
    pub pose: Option<Pose>,
    pub pose_age: Option<f64>,
    pub goal: Option<Goal>,
    pub goal_age: Option<f64>,
    pub goal_observation: Option<String>,
    pub frame_id: Option<String>,
    pub authority: Option<Authority>,
    pub hardware: Option<Hardware>,
    pub reference: Option<Reference>,
}
impl Rover {
    pub fn demo(id: &str, x: f64, y: f64) -> Self {
        Self {
            gps: None,
            navigation: Some(NavigationState::default()),
            id: id.into(),
            membership: "online".into(),
            pose: Some(Pose { x, y, yaw: 0. }),
            pose_age: Some(0.),
            goal: None,
            goal_age: None,
            goal_observation: None,
            frame_id: Some("demo-run".into()),
            authority: Some(Authority {
                reason: None,
                run_id: "demo-run".into(),
                revision: "1".into(),
                requested_level: "teleop".into(),
                supported_levels: vec![
                    "teleop".into(),
                    "assisted_teleop".into(),
                    "waypoint".into(),
                    "waypoint_direct".into(),
                    "target_search".into(),
                ],
                safety: "clear".into(),
                age: 0.,
                token: None,
                result: None,
            }),
            hardware: Some(Hardware {
                arming: Some(false),
                token: None,
                result: None,
                ready: true,
                armed: true,
                reason: "Simulated hardware".into(),
                age: 0.,
            }),
            reference: Some(Reference {
                latitude: 38.8297,
                longitude: -77.3075,
                anchor_x: 0.,
                anchor_y: 0.,
                rotation: 0.,
                frame_id: "demo-run".into(),
            }),
        }
    }
    pub fn ready(&self) -> bool {
        self.membership == "online"
            && self.pose.is_some()
            && fresh(self.pose_age)
            && self
                .hardware
                .as_ref()
                .is_some_and(|h| h.ready && h.armed && fresh(Some(h.age)))
            && self.authority.as_ref().is_some_and(|a| {
                !a.run_id.is_empty()
                    && fresh(Some(a.age))
                    && ["clear", "active"].contains(&a.safety.as_str())
            })
    }
}
pub fn fresh(age: Option<f64>) -> bool {
    age.is_some_and(|v| v.is_finite() && (0.0..argos_core::SUPERVISION_STALE_SECONDS).contains(&v))
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, TS)]
#[serde(tag = "frame", rename_all = "lowercase", deny_unknown_fields)]
pub enum Waypoint {
    Local {
        x: f64,
        y: f64,
        yaw: Option<f64>,
    },
    Geographic {
        latitude: f64,
        longitude: f64,
        yaw: Option<f64>,
    },
}
impl Waypoint {
    pub fn core(&self) -> argos_core::GoalRequest {
        match *self {
            Self::Local { x, y, yaw } => argos_core::GoalRequest::Local { x, y, yaw },
            Self::Geographic {
                latitude,
                longitude,
                yaw,
            } => argos_core::GoalRequest::Geographic {
                latitude,
                longitude,
                yaw,
            },
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, TS)]
#[serde(deny_unknown_fields)]
pub struct WaypointRequest {
    pub run_id: String,
    pub frame_id: Option<String>,
    pub request_id: String,
    pub waypoint: Waypoint,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AutonomyRequest {
    pub request_id: String,
    pub level: String,
    pub run_id: String,
    pub revision: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HardwareRequest {
    pub request_id: String,
    pub action: String,
    pub run_id: String,
    pub revision: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CancelRequest {
    pub request_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Operation {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub settings: Option<NavigationParameters>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub settings_revision: Option<String>,
    pub id: String,
    pub request_id: String,
    pub user_id: String,
    pub owner: String,
    pub rover_id: String,
    pub kind: String,
    pub waypoint: Option<Waypoint>,
    pub phase: String,
    pub message: String,
    pub mode_token: Option<String>,
    pub goal_token: Option<String>,
    pub run_id: String,
    pub goal_observation: Option<String>,
    pub started: f64,
    pub created_at: f64,
    pub mode_level: String,
    pub revision: String,
    pub frame_id: Option<String>,
}
impl Operation {
    pub fn terminal(&self) -> bool {
        [
            "confirmed",
            "arrived",
            "cancelled",
            "rejected",
            "unconfirmed",
            "interrupted",
        ]
        .contains(&self.phase.as_str())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Claim {
    pub owner: String,
    pub login: String,
    pub expires_at: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct FleetSnapshot {
    pub link: String,
    pub rovers: Vec<Rover>,
    pub operations: Vec<Operation>,
    pub claim: Option<Claim>,
    pub now: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct PublicConfig {
    pub demo: bool,
    pub tile_style: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Occupancy {
    pub rover_id: String,
    pub run_id: String,
    pub sequence: String,
    pub frame_id: Option<String>,
    pub width: u32,
    pub height: u32,
    pub resolution: f64,
    pub origin_x: f64,
    pub origin_y: f64,
    pub cells: Vec<i8>,
    pub age: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, TS)]
#[serde(deny_unknown_fields)]
pub struct NavigationParameters {
    pub length: f64,
    pub width: f64,
    pub clearance: f64,
    pub max_linear: f64,
    pub max_angular: f64,
    pub horizon: f64,
}
impl Default for NavigationParameters {
    fn default() -> Self {
        Self {
            length: 0.85,
            width: 0.85,
            clearance: 0.65 - 0.85_f64.hypot(0.85) / 2.,
            max_linear: 2.,
            max_angular: 2.,
            horizon: 2.,
        }
    }
}
impl NavigationParameters {
    pub fn valid(&self) -> bool {
        [
            (self.length, 0.1, 3.),
            (self.width, 0.1, 3.),
            (self.clearance, 0., 1.),
            (self.max_linear, 0.05, 2.),
            (self.max_angular, 0.1, 2.),
            (self.horizon, 0.5, 5.),
        ]
        .iter()
        .all(|&(v, min, max)| v.is_finite() && (min..=max).contains(&v))
    }
    pub fn radius(&self) -> f64 {
        self.length.hypot(self.width) / 2. + self.clearance
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct NavigationState {
    pub schema_version: u32,
    pub revision: String,
    pub parameters: NavigationParameters,
    pub footprint_radius: f64,
    pub age: f64,
    pub token: Option<String>,
    pub result: Option<String>,
}
impl Default for NavigationState {
    fn default() -> Self {
        let p = NavigationParameters::default();
        Self {
            schema_version: 1,
            revision: "0".into(),
            footprint_radius: p.radius(),
            parameters: p,
            age: 0.,
            token: None,
            result: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct NavigationRequest {
    pub request_id: String,
    pub run_id: String,
    pub revision: String,
    pub settings_revision: String,
    pub parameters: NavigationParameters,
}
pub fn navigation_ready(r: &Rover) -> bool {
    let recent = |age: f64| age.is_finite() && (0.0..0.5).contains(&age);
    r.membership == "online"
        && r.hardware
            .as_ref()
            .is_some_and(|h| h.ready && !h.armed && h.arming == Some(false) && recent(h.age))
        && r.authority.as_ref().is_some_and(|a| {
            !a.run_id.is_empty() && recent(a.age) && ["clear", "hold"].contains(&a.safety.as_str())
        })
        && r.navigation
            .as_ref()
            .is_some_and(|n| n.schema_version == 1 && n.parameters.valid() && recent(n.age))
        && r.goal.as_ref().is_none_or(|g| g.state != "active")
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct DriveRequest {
    pub run_id: String,
    pub revision: String,
    #[ts(type = "number")]
    pub sequence: u64,
    pub observed_at: f64,
    pub linear: f64,
    pub angular: f64,
}
