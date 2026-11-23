//! One rover as the console shows it.
//!
//! The rover reports raw messages (`Pose`, `Status`, `TaskStatus`). This
//! module turns them into the two things the operator cares about: what the
//! rover is doing ([`Activity`]) and whether it needs them ([`Urgency`]).

use std::collections::BTreeMap;
use std::time::Duration;

use terra_comms::{
    Attention, Capabilities, Goal, GoalKind, Pose, StageOption, Status, TaskState, TaskStatus,
};

use super::level::Level;

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
    /// Push the base station's saved shared `ARWorldMap` to this rover so it
    /// relocalizes into the fleet's shared world coordinate frame.
    SyncWorldMap,
    /// Ask this rover to capture its current `ARWorldMap` and upload it as the
    /// fleet's shared map.
    CaptureWorldMap,
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
            Self::SyncWorldMap => "Sync map",
            Self::CaptureWorldMap => "Save map",
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
        let stopped = stopped_here || status.is_some_and(|status| status.estopped);
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
            RoverAction::SyncWorldMap | RoverAction::CaptureWorldMap => true,
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
}
