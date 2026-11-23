//! Levels of autonomy: how much of the driving the rover does itself.
//!
//! A level is not something Terra knows about. It is the console's name for
//! a way of commanding the rover, built from what Terra does have: a stick,
//! and tasks that name a controller and a path planner.
//!
//! | Level        | Who steers                    | How it is sent                         |
//! |--------------|-------------------------------|----------------------------------------|
//! | L0 Manual    | the operator                  | stick on `teleop`, no task             |
//! | L1 Assisted  | the operator, slowed and      | task with the `remote` controller; the |
//! |              | stopped in front of obstacles | console filters the stick ([`assist`]) |
//! | L2 Direct    | the rover, straight at the    | goal task: `drive_pose` +              |
//! |              | goal, blind to obstacles      | `straight_line`                        |
//! | L3 Planned   | the rover, around obstacles   | goal task: `dwa` + `hybrid_astar`      |
//! | L4 Explore   | the rover picks its own goals | explore task: `dwa` + `hybrid_astar`   |
//! | L5 Task      | the rover follows a sentence  | not in Terra yet                       |
//!
//! [`assist`]: super::assist

use std::collections::BTreeMap;

use terra_comms::{Capabilities, GoalKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Manual,
    Assisted,
    Direct,
    Planned,
    Explore,
    Task,
}

/// The controller and path planner that make up a level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub controller: String,
    /// `None`: the level uses no path (the operator steers).
    pub planner: Option<&'static str>,
}

impl Level {
    pub const ALL: [Level; 6] = [
        Level::Manual,
        Level::Assisted,
        Level::Direct,
        Level::Planned,
        Level::Explore,
        Level::Task,
    ];

    /// Levels that run a named Terra controller via a task, so the operator
    /// can choose which controller each level uses.
    pub const CONFIGURABLE: [Level; 4] = [
        Level::Assisted,
        Level::Direct,
        Level::Planned,
        Level::Explore,
    ];

    /// "L3".
    pub fn code(self) -> &'static str {
        match self {
            Self::Manual => "L0",
            Self::Assisted => "L1",
            Self::Direct => "L2",
            Self::Planned => "L3",
            Self::Explore => "L4",
            Self::Task => "L5",
        }
    }

    /// "Planned".
    pub fn name(self) -> &'static str {
        match self {
            Self::Manual => "Manual",
            Self::Assisted => "Assisted",
            Self::Direct => "Direct",
            Self::Planned => "Planned",
            Self::Explore => "Explore",
            Self::Task => "Task",
        }
    }

    /// "L3 Planned".
    pub fn label(self) -> String {
        format!("{} {}", self.code(), self.name())
    }

    /// One sentence for the operator.
    pub fn description(self) -> &'static str {
        match self {
            Self::Manual => "You steer. Nothing stops the rover but you.",
            Self::Assisted => "You steer. The rover slows and stops in front of obstacles.",
            Self::Direct => "Drives straight at the goal. Does not avoid obstacles.",
            Self::Planned => "Plans a route to the goal and steers around obstacles.",
            Self::Explore => "Picks its own goals to map the area.",
            Self::Task => "Follows a written instruction.",
        }
    }

    /// Default Terra controller for this level, if it runs one via a task.
    pub fn default_controller(self) -> Option<&'static str> {
        match self {
            Self::Assisted => Some("remote"),
            Self::Direct => Some("drive_pose"),
            Self::Planned | Self::Explore => Some("dwa"),
            Self::Manual | Self::Task => None,
        }
    }

    /// Default Terra path planner for this level, if any.
    pub fn default_planner(self) -> Option<&'static str> {
        match self {
            Self::Direct => Some("straight_line"),
            Self::Planned | Self::Explore => Some("hybrid_astar"),
            Self::Manual | Self::Assisted | Self::Task => None,
        }
    }

    /// The controller configured for this level, falling back to its default.
    pub fn controller_for<'a>(self, overrides: &'a BTreeMap<Level, String>) -> Option<&'a str> {
        if !Self::CONFIGURABLE.contains(&self) {
            return None;
        }
        overrides
            .get(&self)
            .map(String::as_str)
            .or_else(|| self.default_controller())
    }

    /// What Terra has to run for this level with any operator overrides
    /// applied. `None`: the level needs no task (`Manual`), or Terra cannot
    /// run it yet (`Explore`, `Task`).
    pub fn recipe_with(self, overrides: &BTreeMap<Level, String>) -> Option<Recipe> {
        let controller = self.controller_for(overrides)?.to_string();
        Some(Recipe {
            controller,
            planner: self.default_planner(),
        })
    }

    /// The operator steers at this level.
    pub fn is_manual(self) -> bool {
        matches!(self, Self::Manual | Self::Assisted)
    }

    /// The rover drives to a goal clicked on the map at this level.
    pub fn takes_goals(self) -> bool {
        matches!(self, Self::Direct | Self::Planned)
    }

    /// Whether a rover that reports `capabilities` can run this level with
    /// the given controller overrides. Until the rover has reported, only
    /// `Manual` is certain.
    pub fn available_with(
        self,
        capabilities: Option<&Capabilities>,
        overrides: &BTreeMap<Level, String>,
    ) -> bool {
        if self == Self::Manual {
            return true;
        }
        let (Some(recipe), Some(capabilities)) = (self.recipe_with(overrides), capabilities) else {
            return false;
        };
        let has_controller = capabilities
            .controller(&recipe.controller)
            .is_some_and(|stage| match self {
                Self::Explore => stage.accepts(GoalKind::Explore),
                _ if self.takes_goals() => stage.accepts(GoalKind::Local),
                _ => true,
            });
        let has_planner = recipe
            .planner
            .is_none_or(|planner| capabilities.planner(planner).is_some());
        has_controller && has_planner
    }

    /// Whether a rover that reports `capabilities` can run this level using
    /// its default controller.
    pub fn available(self, capabilities: Option<&Capabilities>) -> bool {
        self.available_with(capabilities, &BTreeMap::new())
    }

    /// The level a rover is at, judged by the controller in charge and the
    /// operator's controller assignments. When two goal levels share a
    /// controller, `preferred_goal_level` breaks the tie.
    pub fn of_controller_with(
        controller: &str,
        has_task: bool,
        overrides: &BTreeMap<Level, String>,
        preferred_goal_level: Option<Level>,
    ) -> Self {
        if !has_task || controller.is_empty() || controller == "teleop" {
            return Self::Manual;
        }
        if let Some(level) = preferred_goal_level
            && level.takes_goals()
            && level.controller_for(overrides) == Some(controller)
        {
            return level;
        }
        for level in Self::CONFIGURABLE {
            if level.controller_for(overrides) == Some(controller) {
                return level;
            }
        }
        match controller {
            "remote" => Self::Assisted,
            "drive_pose" => Self::Direct,
            _ => Self::Planned,
        }
    }

    /// The most capable goal level the rover can run, for new goals.
    pub fn best_for_goals_with(
        capabilities: Option<&Capabilities>,
        overrides: &BTreeMap<Level, String>,
    ) -> Self {
        if Self::Planned.available_with(capabilities, overrides) || capabilities.is_none() {
            Self::Planned
        } else {
            Self::Direct
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terra_comms::StageOption;

    fn stage(id: &str, goals: Vec<GoalKind>) -> StageOption {
        StageOption {
            id: id.into(),
            version: "1".into(),
            description: String::new(),
            params: Vec::new(),
            goals,
        }
    }

    #[test]
    fn custom_controller_override_updates_recipe_and_availability() {
        let caps = Capabilities {
            t: 0.0,
            controllers: vec![
                stage("teleop", vec![]),
                stage("pure_pursuit", vec![GoalKind::Local]),
            ],
            planners: vec![stage("hybrid_astar", vec![GoalKind::Local])],
            default_controller: "teleop".into(),
            software: String::new(),
            footprint: None,
        };

        assert!(!Level::Planned.available(Some(&caps)));

        let overrides = BTreeMap::from([(Level::Planned, "pure_pursuit".into())]);
        assert!(Level::Planned.available_with(Some(&caps), &overrides));
        assert_eq!(
            Level::Planned.recipe_with(&overrides),
            Some(Recipe {
                controller: "pure_pursuit".into(),
                planner: Some("hybrid_astar"),
            })
        );
        assert_eq!(
            Level::of_controller_with("pure_pursuit", true, &overrides, None),
            Level::Planned
        );
    }

    #[test]
    fn shared_controller_uses_preferred_goal_level() {
        let overrides = BTreeMap::from([
            (Level::Direct, "dwa".into()),
            (Level::Planned, "dwa".into()),
        ]);
        assert_eq!(
            Level::of_controller_with("dwa", true, &overrides, Some(Level::Direct)),
            Level::Direct
        );
        assert_eq!(
            Level::of_controller_with("dwa", true, &overrides, Some(Level::Planned)),
            Level::Planned
        );
    }
}
