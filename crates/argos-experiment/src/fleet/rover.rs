//! One rover as the console shows it.
//!
//! The rover reports raw messages (`Pose`, `Status`, `TaskStatus`). This
//! module turns them into the two things the operator cares about: what the
//! rover is doing ([`Activity`]) and whether it needs them ([`Urgency`]).

use std::collections::BTreeMap;
use std::time::Duration;

use terra_comms::{
    Attention, Capabilities, Goal, GoalKind, GpsAnchor, Pose, StageOption, Status, TaskState,
    TaskStatus,
};

use super::level::Level;

/// Mean Earth radius (m), matching `terra-av::estimation::gps`.
const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// Default minimum arrival tolerance for GPS goals (m) when anchor accuracy is
/// tighter than this.
pub const MIN_GPS_GOAL_TOLERANCE_M: f64 = 0.35;

/// Rover pose in geographic coordinates (WGS84), derived from `Pose` + `GpsAnchor`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoPose {
    pub latitude: f64,
    pub longitude: f64,
    /// Compass bearing in degrees clockwise from true north `[0, 360)`.
    pub heading_deg: f64,
}

/// Active WGS84 goal (`Goal::Gps`) on a rover.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpsGoal {
    pub latitude: f64,
    pub longitude: f64,
    /// Optional target compass heading (rad, clockwise from true north).
    pub heading: Option<f64>,
    /// Arrival tolerance (m).
    pub tolerance: f64,
}

/// Convert a local world-frame point `(x, y)` (m) into WGS84 `(latitude, longitude)`
/// using `anchor`. Inverse of `terra_av::estimation::GpsAnchor::to_world`.
///
/// `anchor.heading` is radians clockwise from true north to world `+X`;
/// world `+Y` is 90° counter-clockwise (left) of `+X`.
pub fn world_to_lat_lon(anchor: &GpsAnchor, x: f64, y: f64) -> (f64, f64) {
    let (sin_h, cos_h) = anchor.heading.sin_cos();
    let east = x * sin_h - y * cos_h;
    let north = x * cos_h + y * sin_h;
    let d_lat = (north / EARTH_RADIUS_M).to_degrees();
    let cos_lat = anchor.latitude.to_radians().cos().abs().max(1e-6);
    let d_lon = (east / (EARTH_RADIUS_M * cos_lat)).to_degrees();
    (anchor.latitude + d_lat, anchor.longitude + d_lon)
}

/// Convert a world-frame yaw (rad, CCW from world `+X`) to a compass bearing
/// (degrees, CW from true north, normalized to `[0, 360)`).
pub fn world_yaw_to_compass_deg(anchor: &GpsAnchor, yaw: f64) -> f64 {
    (anchor.heading - yaw).to_degrees().rem_euclid(360.0)
}

/// Great-circle (haversine) surface distance in metres between two WGS84 points.
pub fn haversine_distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat * 0.5).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon * 0.5).sin().powi(2);
    2.0 * EARTH_RADIUS_M * a.sqrt().min(1.0).asin()
}

/// What a rover is doing right now.
#[derive(Clone, Debug, PartialEq)]
pub enum Activity {
    /// An e-stop is engaged: ours, the phone's or another base station's.
    Stopped,
    /// The rover gave up on its task and holds still until a human acts.
    Attention(Attention),
    /// At its goal, holding still.
    Arrived,
    /// Paused by an operator.
    Paused,
    /// Steered by an operator through the obstacle assist (level L1).
    HandDriven,
    /// Driving to its goal on its own.
    Driving {
        /// Metres left.
        distance: f64,
    },
    /// No task.
    Idle,
}

/// How badly a rover needs the operator. The more urgent one sorts first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Urgency {
    /// Stopped: nothing happens until someone releases it.
    Stopped,
    /// Waiting for a decision.
    Waiting,
}

/// Something the operator can do with one rover.
///
/// `Focus`, `TakeOver` and `SetGoal` only change what the console shows; the
/// rest are commands, sent by [`super::Fleet::perform`]. `SetLevel` is both:
/// a goal level is a command, a manual level opens the drive view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoverAction {
    /// Show this rover in the focus panel.
    Focus,
    /// Drive it by hand.
    TakeOver,
    /// Pick its next goal on the map.
    SetGoal,
    /// Continue the task after a pause or a problem.
    Resume,
    /// Hold still, keep the task.
    Pause,
    /// Forget the task.
    Clear,
    /// Engage the e-stop.
    Stop,
    /// Release the e-stop this console engaged.
    ReleaseStop,
    /// Change the level of autonomy.
    SetLevel(Level),
    /// Choose which controller to use for an autonomy level.
    SetLevelController { level: Level, controller: String },
    /// Open (`true`) or close (`false`) the rover's dump gate servo.
    SetGate(bool),
}

impl RoverAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Focus => "Show",
            Self::TakeOver => "Take over",
            Self::SetGoal => "Set next goal",
            Self::Resume => "Resume",
            Self::Pause => "Pause",
            Self::Clear => "Clear task",
            Self::Stop => "Stop",
            Self::ReleaseStop => "Release stop",
            Self::SetLevel(level) => level.code(),
            Self::SetLevelController { .. } => "Set controller",
            Self::SetGate(true) => "Open gate",
            Self::SetGate(false) => "Close gate",
        }
    }
}

/// A [`RoverAction`] and the rover it is for. This is what the components
/// hand to their `on_action` listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub rover: u64,
    pub action: RoverAction,
}

#[derive(Clone, Debug)]
pub struct Rover {
    pub id: u64,
    /// Latest pose; `tracked` is false while the rover has lost its position.
    pub pose: Option<Pose>,
    pub activity: Activity,
    /// The level it is at right now, judged by the controller in charge.
    pub level: Level,
    /// The level its next goal will be driven at: the operator's choice, or
    /// the best the rover can do.
    pub goal_level: Level,
    /// The levels this rover's software can run.
    pub available_levels: Vec<Level>,
    /// The controller currently active on the rover (`"teleop"` when idle).
    pub active_controller: String,
    /// Controllers reported by the rover's `Capabilities`. Empty until heard.
    pub controllers: Vec<StageOption>,
    /// Resolved controller id for each configurable autonomy level (L1–L3).
    pub level_controllers: BTreeMap<Level, String>,
    pub task_id: Option<u64>,
    /// Where the task is going, world frame, metres.
    pub goal: Option<[f64; 2]>,
    /// Where the task is going in WGS84 coordinates, when the task has a `Goal::Gps`.
    pub gps_goal: Option<GpsGoal>,
    /// Latest GPS anchor reported by the rover's estimator (`Status.gps_anchor`).
    pub gps_anchor: Option<GpsAnchor>,
    /// Rover position & compass heading in WGS84, derived from tracked `pose` + `gps_anchor`.
    pub geo_pose: Option<GeoPose>,
    /// Why the rover refused the latest request, if it did.
    pub rejected: Option<String>,
    /// This console has an e-stop engaged on it.
    pub stopped_here: bool,
    /// True when the rover reports its dump gate is commanded open.
    pub gate_open: bool,
    /// Physical rover body `[length, width]` in metres (from `Capabilities`,
    /// falling back to `[0.6, 0.45]` on older rovers).
    pub footprint: [f64; 2],
    /// How long it has needed the operator. Zero when it doesn't.
    pub waiting: Duration,
}

/// The latest of each message a rover has sent. Any of them may be missing
/// just after the rover connects.
#[derive(Default)]
pub struct Reports {
    pub pose: Option<Pose>,
    pub status: Option<Status>,
    pub task: Option<TaskStatus>,
    pub capabilities: Option<Capabilities>,
}

impl Rover {
    /// Build the console's view of a rover from its latest messages.
    ///
    /// `stopped_here`, `chosen_goal_level`, and `controller_overrides` are what
    /// the console itself remembers about the rover: its own e-stop, the level
    /// the operator picked for goals, and any per-level controller overrides.
    pub fn interpret(
        id: u64,
        reports: Reports,
        stopped_here: bool,
        chosen_goal_level: Option<Level>,
        controller_overrides: &BTreeMap<Level, String>,
    ) -> Self {
        let Reports {
            pose,
            status,
            task,
            capabilities,
        } = reports;
        let capabilities = capabilities.as_ref();

        let goal_level = chosen_goal_level
            .filter(|level| {
                capabilities.is_none() || level.available_with(capabilities, controller_overrides)
            })
            .unwrap_or_else(|| Level::best_for_goals_with(capabilities, controller_overrides));

        let has_task = task.as_ref().is_some_and(|task| task.task_id.is_some());
        let controller = task
            .as_ref()
            .map(|task| task.controller.as_str())
            .unwrap_or_default();
        let active_controller = if has_task && !controller.is_empty() {
            controller.to_string()
        } else {
            capabilities
                .map(|caps| caps.default_controller.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "teleop".into())
        };
        let is_explore = has_task
            && task
                .as_ref()
                .is_some_and(|t| matches!(t.goal, Goal::Explore { .. }));
        let level = if is_explore {
            Level::Explore
        } else {
            Level::of_controller_with(controller, has_task, controller_overrides, Some(goal_level))
        };

        let gate_open = status.as_ref().is_some_and(|status| status.gate_open);
        let stopped = stopped_here || status.as_ref().is_some_and(|status| status.estopped);
        let activity = match task.as_ref().map(|task| &task.state) {
            _ if stopped => Activity::Stopped,
            Some(TaskState::NeedsAttention { reason, .. }) => Activity::Attention(reason.clone()),
            Some(TaskState::Reached) => Activity::Arrived,
            Some(TaskState::Paused) => Activity::Paused,
            Some(TaskState::Running { .. }) if level == Level::Assisted => Activity::HandDriven,
            Some(TaskState::Running {
                distance_to_goal, ..
            }) => Activity::Driving {
                distance: *distance_to_goal,
            },
            Some(TaskState::Idle) | None => Activity::Idle,
        };
        let goal = task
            .as_ref()
            .filter(|_| has_task)
            .and_then(|task| match task.goal {
                Goal::Pose { x, y, .. } => Some([x, y]),
                Goal::Stop | Goal::Gps { .. } | Goal::Explore { .. } => None,
            });
        let gps_goal = task
            .as_ref()
            .filter(|_| has_task)
            .and_then(|task| match task.goal {
                Goal::Gps {
                    latitude,
                    longitude,
                    heading,
                    tolerance,
                } => Some(GpsGoal {
                    latitude,
                    longitude,
                    heading,
                    tolerance,
                }),
                Goal::Stop | Goal::Pose { .. } | Goal::Explore { .. } => None,
            });
        let gps_anchor = status.as_ref().and_then(|s| s.gps_anchor.clone());
        let geo_pose = match (pose.as_ref(), gps_anchor.as_ref()) {
            (Some(pose), Some(anchor)) if pose.tracked => {
                let (latitude, longitude) = world_to_lat_lon(anchor, pose.x, pose.y);
                let heading_deg = world_yaw_to_compass_deg(anchor, pose.yaw);
                Some(GeoPose {
                    latitude,
                    longitude,
                    heading_deg,
                })
            }
            _ => None,
        };

        let level_controllers = Level::CONFIGURABLE
            .into_iter()
            .filter_map(|lvl| {
                lvl.controller_for(controller_overrides)
                    .map(|name| (lvl, name.to_string()))
            })
            .collect();

        Self {
            id,
            pose,
            activity,
            level,
            // The operator's choice stands unless the rover has said it
            // cannot run it. (A rover repeats what it can run only every
            // few seconds, so just after start-up that is still unknown.)
            goal_level,
            available_levels: Level::ALL
                .into_iter()
                .filter(|level| level.available_with(capabilities, controller_overrides))
                .collect(),
            active_controller,
            controllers: capabilities
                .map(|caps| caps.controllers.clone())
                .unwrap_or_default(),
            level_controllers,
            task_id: task.as_ref().and_then(|task| task.task_id),
            goal,
            gps_goal,
            gps_anchor,
            geo_pose,
            rejected: task
                .and_then(|task| task.last_rejected)
                .map(|rejection| rejection.reason),
            stopped_here,
            gate_open,
            footprint: capabilities
                .and_then(|caps| caps.footprint)
                .unwrap_or([0.6, 0.45]),
            waiting: Duration::ZERO,
        }
    }

    /// The controller configured for `level`, if it is a configurable level.
    pub fn controller_for(&self, level: Level) -> Option<&str> {
        self.level_controllers.get(&level).map(String::as_str)
    }

    /// Whether this rover can accept a `Goal::Gps` at its current `goal_level`.
    ///
    /// Follows `terra-comms` API §4.3:
    /// - A controller listing `GoalKind::Gps` accepts GPS goals directly.
    /// - A controller listing `GoalKind::Local` accepts GPS goals once the
    ///   rover reports a `gps_anchor` (the rover converts `Goal::Gps` to local).
    pub fn supports_gps_goal(&self) -> Result<(), &'static str> {
        let controller_name = self.controller_for(self.goal_level);
        let stage = controller_name
            .and_then(|id| self.controllers.iter().find(|c| c.id == id));

        if let Some(stage) = stage {
            if stage.accepts(GoalKind::Gps) {
                return Ok(());
            }
            if !stage.accepts(GoalKind::Local) {
                return Err("Selected controller does not accept GPS goals");
            }
        }
        if self.gps_anchor.is_some() {
            Ok(())
        } else {
            Err("No GPS anchor yet (drive ~10 m outdoors with GPS)")
        }
    }

    /// Arrival tolerance (m) for a GPS goal sent to this rover: never tighter
    /// than the rover's current `gps_anchor.accuracy` (`terra-comms` API §6).
    pub fn gps_goal_tolerance(&self) -> f64 {
        let accuracy = self
            .gps_anchor
            .as_ref()
            .map_or(0.0, |anchor| if anchor.accuracy.is_finite() { anchor.accuracy } else { 0.0 });
        MIN_GPS_GOAL_TOLERANCE_M.max(accuracy)
    }

    /// "Rover 4".
    pub fn name(&self) -> String {
        format!("Rover {}", self.id)
    }

    /// "R4", for the map.
    pub fn short_name(&self) -> String {
        format!("R{}", self.id)
    }

    /// `None`: the rover is fine on its own.
    pub fn urgency(&self) -> Option<Urgency> {
        match self.activity {
            Activity::Stopped => Some(Urgency::Stopped),
            Activity::Attention(_) | Activity::Arrived => Some(Urgency::Waiting),
            Activity::Paused | Activity::HandDriven | Activity::Driving { .. } | Activity::Idle => {
                None
            }
        }
    }

    /// A few words: what the rover is doing.
    pub fn headline(&self) -> &'static str {
        match &self.activity {
            Activity::Stopped => "E-stop",
            Activity::Attention(reason) => match reason {
                Attention::Timeout => "Timed out",
                Attention::Stuck => "Stuck",
                Attention::ControllerError(_) => "Controller error",
                Attention::PoseLost => "Lost its position",
                Attention::NoGpsAnchor => "No GPS anchor",
                Attention::EStop => "Was stopped",
                Attention::RemoteSilent => "Assist lost the console",
                Attention::Displaced => "Pushed off its goal",
                Attention::RoverRestarted => "Restarted",
            },
            Activity::Arrived => "Arrived at goal",
            Activity::Paused => "Paused",
            Activity::HandDriven => "Driven by hand",
            Activity::Driving { .. } => "Driving to goal",
            Activity::Idle => "Idle",
        }
    }

    /// One or two sentences: what happened and what it means.
    pub fn explanation(&self) -> String {
        match &self.activity {
            Activity::Stopped if self.stopped_here => {
                "Stopped from this console. It stays stopped until you release it.".into()
            }
            Activity::Stopped => {
                "Stopped on the phone or by another base station. Only they can release it.".into()
            }
            Activity::Attention(reason) => match reason {
                Attention::Timeout => {
                    "The goal took longer than the controller allows. Holding still.".into()
                }
                Attention::Stuck => "No progress toward the goal. Holding still.".into(),
                Attention::ControllerError(error) => format!("The controller failed: {error}"),
                Attention::PoseLost => "The rover does not know where it is. Holding still.".into(),
                Attention::NoGpsAnchor => {
                    "Its GPS goal can no longer be placed in the world. Holding still.".into()
                }
                Attention::EStop => {
                    "An e-stop interrupted the task. It does not continue by itself.".into()
                }
                Attention::RemoteSilent => {
                    "The obstacle assist stopped hearing from the console. Holding still.".into()
                }
                Attention::Displaced => {
                    "It was pushed away from its goal and will not drive back alone.".into()
                }
                Attention::RoverRestarted => {
                    "The rover restarted and kept its task. It waits for a go.".into()
                }
            },
            Activity::Arrived => "Holding still at the goal. Waiting for the next one.".into(),
            Activity::Paused => "Holding still. The task and its goal are kept.".into(),
            Activity::HandDriven => "An operator steers; the obstacle assist is on.".into(),
            Activity::Driving { distance } => format!("{distance:.1} m from its goal."),
            Activity::Idle => "No task. Set a goal or take over.".into(),
        }
    }

    /// The two things the operator most likely wants when this rover needs
    /// them: the main one first.
    pub fn suggested_actions(&self) -> Option<(RoverAction, RoverAction)> {
        match self.activity {
            Activity::Stopped => Some((RoverAction::ReleaseStop, RoverAction::TakeOver)),
            Activity::Attention(_) => Some((RoverAction::Resume, RoverAction::TakeOver)),
            Activity::Arrived => Some((RoverAction::SetGoal, RoverAction::Clear)),
            Activity::Paused | Activity::HandDriven | Activity::Driving { .. } | Activity::Idle => {
                None
            }
        }
    }

    /// Whether `action` makes sense right now. The rover has the last word:
    /// this only greys out buttons that would be refused anyway.
    pub fn allows(&self, action: &RoverAction) -> bool {
        match action {
            RoverAction::Focus | RoverAction::TakeOver | RoverAction::SetGoal => true,
            RoverAction::Resume => {
                matches!(self.activity, Activity::Attention(_) | Activity::Paused)
            }
            RoverAction::Pause => matches!(self.activity, Activity::Driving { .. }),
            RoverAction::Clear => self.task_id.is_some(),
            RoverAction::Stop => !self.stopped_here,
            RoverAction::ReleaseStop => self.stopped_here,
            RoverAction::SetLevel(level) => self.available_levels.contains(level),
            RoverAction::SetLevelController { level, controller } => {
                Level::CONFIGURABLE.contains(level)
                    && self
                        .controllers
                        .iter()
                        .find(|stage| stage.id == *controller)
                        .is_some_and(|stage| match level {
                            Level::Explore => stage.accepts(GoalKind::Explore),
                            _ if level.takes_goals() => stage.accepts(GoalKind::Local),
                            _ => true,
                        })
            }
            RoverAction::SetGate(_) => !matches!(self.activity, Activity::Stopped),
        }
    }
}

/// The rovers that need the operator: most urgent first, then longest wait.
pub fn help_queue(rovers: &[Rover]) -> Vec<Rover> {
    let mut queue: Vec<Rover> = rovers
        .iter()
        .filter(|rover| rover.urgency().is_some())
        .cloned()
        .collect();
    queue.sort_by(|a, b| {
        a.urgency()
            .cmp(&b.urgency())
            .then(b.waiting.cmp(&a.waiting))
    });
    queue
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rover(id: u64, activity: Activity, waiting: u64) -> Rover {
        Rover {
            id,
            pose: None,
            activity,
            level: Level::Planned,
            goal_level: Level::Planned,
            available_levels: Vec::new(),
            active_controller: "dwa".into(),
            controllers: Vec::new(),
            level_controllers: BTreeMap::new(),
            task_id: Some(1),
            goal: None,
            gps_goal: None,
            gps_anchor: None,
            geo_pose: None,
            rejected: None,
            stopped_here: false,
            gate_open: false,
            footprint: [0.6, 0.45],
            waiting: Duration::from_secs(waiting),
        }
    }

    #[test]
    fn queue_puts_stopped_first_then_longest_wait() {
        let rovers = [
            rover(1, Activity::Driving { distance: 2.0 }, 0),
            rover(2, Activity::Arrived, 10),
            rover(3, Activity::Attention(Attention::Stuck), 40),
            rover(4, Activity::Stopped, 5),
        ];
        let order: Vec<u64> = help_queue(&rovers).iter().map(|rover| rover.id).collect();
        assert_eq!(order, [4, 3, 2]);
    }

    #[test]
    fn the_chosen_goal_level_survives_until_the_rover_says_what_it_can_run() {
        let none = BTreeMap::new();
        let unknown = Rover::interpret(1, Reports::default(), false, Some(Level::Direct), &none);
        assert_eq!(unknown.goal_level, Level::Direct);

        // This rover has no `drive_pose`, so L2 is not on offer.
        let only_dwa = Capabilities {
            t: 0.0,
            controllers: vec![stage("dwa")],
            planners: vec![stage("hybrid_astar")],
            default_controller: "teleop".into(),
            software: String::new(),
            footprint: None,
        };
        let reports = Reports {
            capabilities: Some(only_dwa),
            ..Default::default()
        };
        let known = Rover::interpret(1, reports, false, Some(Level::Direct), &none);
        assert_eq!(known.goal_level, Level::Planned);
        assert_eq!(known.available_levels, [Level::Manual, Level::Planned]);
    }

    #[test]
    fn controller_override_makes_level_available_and_tracks_controllers() {
        let caps = Capabilities {
            t: 0.0,
            controllers: vec![
                terra_comms::StageOption {
                    id: "teleop".into(),
                    version: "1".into(),
                    description: "Stick".into(),
                    params: Vec::new(),
                    goals: Vec::new(),
                },
                stage("mppi"),
            ],
            planners: vec![stage("hybrid_astar")],
            default_controller: "teleop".into(),
            software: String::new(),
            footprint: None,
        };
        let overrides = BTreeMap::from([(Level::Planned, "mppi".into())]);
        let rover = Rover::interpret(
            1,
            Reports {
                capabilities: Some(caps),
                ..Default::default()
            },
            false,
            None,
            &overrides,
        );
        assert!(rover.available_levels.contains(&Level::Planned));
        assert_eq!(rover.controller_for(Level::Planned), Some("mppi"));
        assert_eq!(rover.controllers.len(), 2);
        assert!(rover.allows(&RoverAction::SetLevelController {
            level: Level::Planned,
            controller: "mppi".into(),
        }));
        assert!(!rover.allows(&RoverAction::SetLevelController {
            level: Level::Planned,
            controller: "teleop".into(),
        }));
    }

    fn stage(id: &str) -> terra_comms::StageOption {
        terra_comms::StageOption {
            id: id.into(),
            version: "1".into(),
            description: String::new(),
            params: Vec::new(),
            goals: vec![terra_comms::GoalKind::Local],
        }
    }

    #[test]
    fn an_estop_wins_over_the_task_state() {
        let status = Status {
            t: 0.0,
            estopped: true,
            link_ok: true,
            detail: String::new(),
            gps_anchor: None,
            gate_open: false,
        };
        let reports = Reports {
            status: Some(status),
            ..Default::default()
        };
        let rover = Rover::interpret(7, reports, false, None, &BTreeMap::new());
        assert_eq!(rover.activity, Activity::Stopped);
        assert!(
            !rover.allows(&RoverAction::ReleaseStop),
            "engaged elsewhere: not ours to release"
        );
        assert!(!rover.allows(&RoverAction::Resume));
    }

    #[test]
    fn world_to_lat_lon_and_compass_heading_match_anchor_convention() {
        // World +X points due east (heading = 90° CW from north).
        // World +Y is 90° CCW (left) of +X, so world +Y points due north.
        let anchor = GpsAnchor {
            latitude: 37.7749,
            longitude: -122.4194,
            heading: std::f64::consts::FRAC_PI_2,
            accuracy: 2.0,
        };
        let (lat_x, lon_x) = world_to_lat_lon(&anchor, 100.0, 0.0);
        assert!((lat_x - anchor.latitude).abs() < 1e-9);
        assert!(lon_x > anchor.longitude);

        let (lat_y, lon_y) = world_to_lat_lon(&anchor, 0.0, 100.0);
        assert!(lat_y > anchor.latitude);
        assert!((lon_y - anchor.longitude).abs() < 1e-9);

        // Distance for 100 m north should be ~100 m.
        let d = haversine_distance_m(anchor.latitude, anchor.longitude, lat_y, lon_y);
        assert!((d - 100.0).abs() < 0.2, "expected ~100 m, got {d}");

        // Yaw = 0 (facing +X = east) -> compass 90°.
        assert!((world_yaw_to_compass_deg(&anchor, 0.0) - 90.0).abs() < 1e-6);
        // Yaw = +90° (facing +Y = north) -> compass 0°.
        let north = world_yaw_to_compass_deg(&anchor, std::f64::consts::FRAC_PI_2);
        assert!(north.abs() < 1e-6 || (north - 360.0).abs() < 1e-6);
    }

    #[test]
    fn gps_goal_support_and_tolerance_respect_anchor_and_capabilities() {
        let caps = Capabilities {
            t: 0.0,
            controllers: vec![stage("dwa")],
            planners: vec![stage("hybrid_astar")],
            default_controller: "teleop".into(),
            software: String::new(),
            footprint: None,
        };
        let unanchored = Rover::interpret(
            1,
            Reports {
                capabilities: Some(caps.clone()),
                ..Default::default()
            },
            false,
            None,
            &BTreeMap::new(),
        );
        assert!(unanchored.supports_gps_goal().is_err());

        let anchor = GpsAnchor {
            latitude: 37.7749,
            longitude: -122.4194,
            heading: 0.0,
            accuracy: 2.4,
        };
        let anchored = Rover::interpret(
            1,
            Reports {
                capabilities: Some(caps),
                status: Some(Status {
                    t: 0.0,
                    estopped: false,
                    link_ok: true,
                    detail: String::new(),
                    gps_anchor: Some(anchor),
                    gate_open: false,
                }),
                pose: Some(Pose {
                    t: 0.0,
                    x: 10.0,
                    y: 0.0,
                    yaw: 0.0,
                    tracked: true,
                }),
                ..Default::default()
            },
            false,
            None,
            &BTreeMap::new(),
        );
        assert!(anchored.supports_gps_goal().is_ok());
        assert!((anchored.gps_goal_tolerance() - 2.4).abs() < 1e-9);
        assert!(anchored.geo_pose.is_some());
    }
}
