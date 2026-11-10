//! Operator-planned missions: a list of steps one rover works through, built
//! only from commands Terra already takes (L2/L3 waypoint goals and the bucket
//! dock). Nothing here touches the network; the window ticks a
//! [`MissionRun`] with what it sees of the rover and sends the
//! [`MissionCommand`]s it returns.
//!
//! Steps:
//! - `Waypoint`: drive to a point.
//! - `Collect`: drive to a point to pick up balls on the way. Same command as
//!   a waypoint; the kind marks where "Out of balls" sends the rover back to.
//! - `Home`: drive to the home goal, then wait for the human to refill the
//!   dumper. "Loaded" continues; "Out of balls" jumps to the first `Collect`.
//! - `Score`: drive toward a found bucket, facing it, and start the dock as
//!   soon as the rover is within [`DOCK_START_M`] and facing it (or when the
//!   goal arrives). The dump itself stays the operator's approval (L0–L3).

use crate::DockReport;
use serde::{Deserialize, Serialize};

/// Levels that take a local waypoint goal.
pub const MISSION_LEVELS: [&str; 2] = ["waypoint", "waypoint_direct"];
/// A score step starts the dock once the rover is this close to the bucket
/// centre and facing it, without waiting for its goal to arrive. Terra's dock
/// sees the bucket from here: past its blind range (about 0.96 m on Terra, a
/// dock started inside it reverses out to 1.11 m first) and short of the 1.7 m
/// where field run b1999e3e got no camera fix at all. Well inside the dock's
/// 3 m start limit.
pub const DOCK_START_M: f64 = 1.35;
/// "Facing the bucket" for [`DOCK_START_M`]: bucket bearing from the rover's
/// heading. Terra's dock backs off above 25°.
pub const DOCK_START_BEARING_DEG: f64 = 20.;
/// The score step's goal sits this far short of the bucket centre, facing it.
/// Terra counts an L2/L3 goal as arrived 0.75 m out, so a straight approach
/// stops about 1.25 m from the bucket, inside [`DOCK_START_M`]; the mission
/// usually starts the dock a moment before that. Terra slides a goal that
/// would put the body into the bucket back to where it fits.
pub const SCORE_GOAL_M: f64 = 0.5;
/// A goal or dock the rover has not taken after this long is sent again.
pub const TAKE_TIMEOUT_S: f64 = 5.;
/// Sends per step before the mission pauses.
pub const MAX_SENDS: u32 = 3;
/// A reported goal within this of the step's target is the step's goal.
const SAME_GOAL_M: f64 = 0.3;
/// Longest plan the editor accepts.
pub const MAX_STEPS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Waypoint,
    Collect,
    Home,
    Score,
}

impl StepKind {
    pub const PLACEABLE: [Self; 3] = [Self::Waypoint, Self::Collect, Self::Home];
    pub fn label(self) -> &'static str {
        match self {
            Self::Waypoint => "Waypoint",
            Self::Collect => "Collect",
            Self::Home => "Home",
            Self::Score => "Score",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Waypoint => "waypoint",
            Self::Collect => "collect",
            Self::Home => "home",
            Self::Score => "score",
        }
    }
    /// Map marker colour, RGB.
    pub fn color(self) -> u32 {
        match self {
            Self::Waypoint => 0xb8c2cc,
            Self::Collect => 0x4fbf6a,
            Self::Home => 0x5b9cf0,
            Self::Score => 0xf08a3c,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MissionStep {
    pub kind: StepKind,
    /// Local map position, metres. For `Score`, the bucket's position when the
    /// step was planned; the live object position is used when it is known.
    pub x: f64,
    pub y: f64,
    /// Found bucket id, `Score` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<u32>,
    /// "Orange bucket 4", `Score` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket_label: Option<String>,
}

impl MissionStep {
    pub fn at(kind: StepKind, x: f64, y: f64) -> Self {
        Self {
            kind,
            x,
            y,
            bucket: None,
            bucket_label: None,
        }
    }
    pub fn score(bucket: u32, label: String, x: f64, y: f64) -> Self {
        Self {
            kind: StepKind::Score,
            x,
            y,
            bucket: Some(bucket),
            bucket_label: Some(label),
        }
    }
    /// "Collect (2.0, -4.5)" or "Score at Orange bucket 4".
    pub fn describe(&self) -> String {
        match (&self.kind, &self.bucket_label) {
            (StepKind::Score, Some(label)) => format!("Score at {label}"),
            _ => format!("{} ({:.1}, {:.1})", self.kind.label(), self.x, self.y),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MissionPlan {
    pub steps: Vec<MissionStep>,
    /// Start again from step 1 after the last step.
    #[serde(default)]
    pub repeat: bool,
}

impl MissionPlan {
    pub fn push(&mut self, step: MissionStep) -> Result<(), &'static str> {
        if self.steps.len() >= MAX_STEPS {
            return Err("A mission holds at most 64 steps");
        }
        if !step.x.is_finite() || !step.y.is_finite() {
            return Err("That point has no local position");
        }
        self.steps.push(step);
        Ok(())
    }
    pub fn remove(&mut self, index: usize) {
        if index < self.steps.len() {
            self.steps.remove(index);
        }
    }
    /// Swap step `index` with the one before it.
    pub fn move_up(&mut self, index: usize) {
        if index > 0 && index < self.steps.len() {
            self.steps.swap(index - 1, index);
        }
    }
    pub fn move_down(&mut self, index: usize) {
        if index + 1 < self.steps.len() {
            self.steps.swap(index, index + 1);
        }
    }
    fn first_collect(&self) -> Option<usize> {
        self.steps.iter().position(|s| s.kind == StepKind::Collect)
    }
}

/// What the window sees of the rover this tick.
#[derive(Clone, Copy, Debug)]
pub struct MissionView<'a> {
    pub x: f64,
    pub y: f64,
    /// Heading in the local frame, radians, 0 facing +x.
    pub yaw: f64,
    /// Effective level, else the requested one.
    pub level: Option<&'a str>,
    /// Reported goal: x, y and `active` / `arrived` / `idle`.
    pub goal: Option<(f64, f64, &'a str)>,
    pub dock: Option<&'a DockReport>,
    /// Live position of found buckets: (id, x, y).
    pub buckets: &'a [(u32, f64, f64)],
    /// Emergency stop latched, or pose not fresh.
    pub halted: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MissionCommand {
    Goal {
        x: f64,
        y: f64,
        yaw: Option<f64>,
    },
    Dock {
        bucket: u32,
    },
    /// Stop where the rover is: a goal at its own position. Terra drops L2/L3
    /// to L0 on a goal cancel, so the mission never cancels a goal; a hold
    /// keeps the level and a new goal can follow.
    Hold {
        x: f64,
        y: f64,
    },
    CancelDock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    /// At home: the human loads the dumper.
    Refill,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Goal sent for the current step; `seen` once the rover reports it active.
    Driving {
        sent_at: f64,
        sends: u32,
        seen: bool,
        target: (f64, f64),
    },
    /// Dock sent; `dock_id` once the rover reports a dock on this bucket.
    Docking {
        sent_at: f64,
        sends: u32,
        dock_id: Option<u64>,
    },
    Waiting(Wait),
    Paused(String),
    Done,
}

/// One rover's running mission.
#[derive(Clone, Debug, PartialEq)]
pub struct MissionRun {
    pub plan: MissionPlan,
    pub index: usize,
    pub phase: Phase,
    /// Completed passes through the plan (with `repeat`).
    pub laps: u32,
    /// Step to start when resumed from a pause.
    resume_docking: bool,
}

/// A change worth a session-log line.
#[derive(Clone, Debug, PartialEq)]
pub struct MissionEvent {
    pub index: usize,
    pub kind: StepKind,
    /// `started`, `arrived`, `in_dock_range`, `docking`, `scored`, `waiting_refill`, `paused`,
    /// `resumed`, `loaded`, `out_of_balls`, `skipped`, `done`, `stopped`.
    pub event: &'static str,
    pub detail: Option<String>,
}

/// What a tick asks for: commands to send and events to log.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MissionTick {
    pub commands: Vec<MissionCommand>,
    pub events: Vec<MissionEvent>,
}

/// Why a mission cannot start (or resume) on this rover now.
pub fn mission_block(view: &MissionView) -> Option<String> {
    if let Some(reason) = view.halted {
        return Some(reason.to_string());
    }
    match view.level {
        Some(level) if MISSION_LEVELS.contains(&level) => None,
        Some(level) => Some(format!(
            "Missions drive with waypoint goals: set L2 Direct or L3 Planned (rover is at {})",
            crate::level_label(level)
        )),
        None => Some("Rover has not reported a level yet".into()),
    }
}

impl MissionRun {
    /// Start `plan` at step 0. Errors when the plan is empty or the rover
    /// cannot take goals now.
    pub fn start(
        plan: MissionPlan,
        view: &MissionView,
        now: f64,
    ) -> Result<(Self, MissionTick), String> {
        if plan.steps.is_empty() {
            return Err("Add at least one step".into());
        }
        if let Some(reason) = mission_block(view) {
            return Err(reason);
        }
        let mut run = Self {
            plan,
            index: 0,
            phase: Phase::Done,
            laps: 0,
            resume_docking: false,
        };
        let mut tick = MissionTick::default();
        run.enter(0, view, now, &mut tick);
        Ok((run, tick))
    }

    pub fn current(&self) -> Option<&MissionStep> {
        self.plan.steps.get(self.index)
    }

    pub fn active(&self) -> bool {
        !matches!(self.phase, Phase::Done)
    }

    /// One line for the panel and the rover card.
    pub fn status(&self) -> String {
        let total = self.plan.steps.len();
        let step = self
            .current()
            .map(MissionStep::describe)
            .unwrap_or_default();
        let lap = if self.plan.repeat && self.laps > 0 {
            format!(" · lap {}", self.laps + 1)
        } else {
            String::new()
        };
        let n = self.index + 1;
        match &self.phase {
            Phase::Driving { seen, .. } => format!(
                "Step {n}/{total}: {step} · {}{lap}",
                if *seen { "driving" } else { "sending goal" }
            ),
            Phase::Docking { dock_id, .. } => format!(
                "Step {n}/{total}: {step} · {}{lap}",
                if dock_id.is_some() {
                    "docking"
                } else {
                    "starting dock"
                }
            ),
            Phase::Waiting(Wait::Refill) => {
                format!("Step {n}/{total}: at home · waiting for refill{lap}")
            }
            Phase::Paused(reason) => format!("Paused at step {n}/{total}: {reason}"),
            Phase::Done => format!("Mission done · {total} steps"),
        }
    }

    /// Advance with what the rover reports now.
    pub fn tick(&mut self, view: &MissionView, now: f64) -> MissionTick {
        let mut tick = MissionTick::default();
        if matches!(
            self.phase,
            Phase::Done | Phase::Paused(_) | Phase::Waiting(_)
        ) {
            return tick;
        }
        if let Some(reason) = mission_block(view) {
            self.pause(reason, &mut tick);
            return tick;
        }
        let Some(step) = self.current().cloned() else {
            self.finish(&mut tick);
            return tick;
        };
        match self.phase.clone() {
            Phase::Driving {
                sent_at,
                sends,
                seen,
                target,
            } => {
                if step.kind == StepKind::Score && dock_window(&step, view) {
                    // Close enough and facing the bucket: dock now. The goal
                    // stays; it sits short of the bucket, so the rover is at
                    // or inside its arrival radius when the dock ends.
                    self.log(&mut tick, "in_dock_range", None);
                    self.start_dock(view, now, &mut tick);
                    return tick;
                }
                let ours = view
                    .goal
                    .filter(|(x, y, _)| (x - target.0).hypot(y - target.1) <= SAME_GOAL_M);
                match ours.map(|(_, _, state)| state) {
                    Some("arrived") => self.arrived(&step, view, now, &mut tick),
                    Some("active") => {
                        if !seen {
                            self.phase = Phase::Driving {
                                sent_at,
                                sends,
                                seen: true,
                                target,
                            };
                        }
                    }
                    _ if seen => self.pause(
                        "Goal ended before arriving · check the rover's hold reason".into(),
                        &mut tick,
                    ),
                    _ if now - sent_at >= TAKE_TIMEOUT_S => {
                        if sends >= MAX_SENDS {
                            self.pause("Rover did not take the goal".into(), &mut tick);
                        } else {
                            tick.commands.push(MissionCommand::Goal {
                                x: target.0,
                                y: target.1,
                                yaw: goal_yaw(&step, view, target),
                            });
                            self.phase = Phase::Driving {
                                sent_at: now,
                                sends: sends + 1,
                                seen: false,
                                target,
                            };
                        }
                    }
                    _ => {}
                }
            }
            Phase::Docking {
                sent_at,
                sends,
                dock_id,
            } => {
                let bucket = step.bucket;
                let report = view
                    .dock
                    .filter(|dock| dock.object_id.is_some() && dock.object_id == bucket);
                match (dock_id, report) {
                    (None, Some(dock)) if dock.active() => {
                        self.phase = Phase::Docking {
                            sent_at,
                            sends,
                            dock_id: Some(dock.dock_id),
                        };
                        if dock.phase == "done" {
                            self.scored(view, now, &mut tick);
                        }
                    }
                    (Some(id), Some(dock)) if dock.dock_id == id => match dock.phase.as_str() {
                        "done" => self.scored(view, now, &mut tick),
                        "failed" | "cancelled" => {
                            // A dock started before the goal arrived leaves
                            // that goal live; hold so the rover does not drive
                            // on toward the bucket.
                            if view.goal.is_some_and(|(_, _, state)| state == "active") {
                                tick.commands.push(MissionCommand::Hold {
                                    x: view.x,
                                    y: view.y,
                                });
                            }
                            self.pause(format!("Dock {}: {}", dock.phase, dock.reason), &mut tick)
                        }
                        _ => {}
                    },
                    (Some(_), _) => self.pause("Dock ended without a dump".into(), &mut tick),
                    (None, _) if now - sent_at >= TAKE_TIMEOUT_S => {
                        if sends >= MAX_SENDS {
                            self.pause(
                                "Rover did not start the dock · is the bucket in view?".into(),
                                &mut tick,
                            );
                        } else if let Some(bucket) = bucket {
                            tick.commands.push(MissionCommand::Dock { bucket });
                            self.phase = Phase::Docking {
                                sent_at: now,
                                sends: sends + 1,
                                dock_id: None,
                            };
                        }
                    }
                    _ => {}
                }
            }
            Phase::Waiting(_) | Phase::Paused(_) | Phase::Done => {}
        }
        tick
    }

    /// The human loaded the dumper: go on to the next step.
    pub fn loaded(&mut self, view: &MissionView, now: f64) -> MissionTick {
        let mut tick = MissionTick::default();
        if self.phase == Phase::Waiting(Wait::Refill) {
            self.log(&mut tick, "loaded", None);
            self.next(self.index + 1, view, now, &mut tick);
        }
        tick
    }

    /// No balls left at home: go back to the first Collect step.
    pub fn out_of_balls(&mut self, view: &MissionView, now: f64) -> MissionTick {
        let mut tick = MissionTick::default();
        if self.phase == Phase::Waiting(Wait::Refill) {
            self.log(&mut tick, "out_of_balls", None);
            let to = self.plan.first_collect().unwrap_or(self.index + 1);
            self.next(to, view, now, &mut tick);
        }
        tick
    }

    /// Pause now. With the rover's view, a goal in flight is held where the
    /// rover is and a dock is cancelled; without one (rover gone) nothing is sent.
    pub fn pause_now(&mut self, why: &str, view: Option<&MissionView>) -> MissionTick {
        let mut tick = MissionTick::default();
        if let Some(view) = view {
            self.halt_motion(view, &mut tick);
        }
        if !matches!(self.phase, Phase::Done | Phase::Paused(_)) {
            self.pause(why.to_string(), &mut tick);
        }
        tick
    }

    /// Re-run the current step from its start.
    pub fn resume(&mut self, view: &MissionView, now: f64) -> Result<MissionTick, String> {
        if !matches!(self.phase, Phase::Paused(_)) {
            return Ok(MissionTick::default());
        }
        if let Some(reason) = mission_block(view) {
            return Err(reason);
        }
        let mut tick = MissionTick::default();
        self.log(&mut tick, "resumed", None);
        if self.resume_docking {
            self.start_dock(view, now, &mut tick);
        } else {
            self.enter(self.index, view, now, &mut tick);
        }
        Ok(tick)
    }

    /// Drop the current step and start the next one (paused if the rover cannot drive).
    pub fn skip(&mut self, view: &MissionView, now: f64) -> MissionTick {
        let mut tick = MissionTick::default();
        if !self.active() {
            return tick;
        }
        // A dock has to end before the next goal; a goal in flight is simply
        // replaced by the next step's goal (never cancelled, see `Hold`).
        if matches!(self.phase, Phase::Docking { .. }) {
            tick.commands.push(MissionCommand::CancelDock);
        }
        self.log(&mut tick, "skipped", None);
        let last = self.index + 1 >= self.plan.steps.len() && !self.plan.repeat;
        if let Some(reason) = mission_block(view) {
            self.halt_driving(view, &mut tick);
            self.index = (self.index + 1).min(self.plan.steps.len());
            self.resume_docking = false;
            if self.index >= self.plan.steps.len() {
                self.finish(&mut tick);
            } else {
                self.pause(reason, &mut tick);
            }
        } else {
            if last {
                self.halt_driving(view, &mut tick);
            }
            self.next(self.index + 1, view, now, &mut tick);
        }
        tick
    }

    /// End the mission; holds the goal or cancels the dock it had in flight.
    pub fn stop(&mut self, view: Option<&MissionView>) -> MissionTick {
        let mut tick = MissionTick::default();
        if let Some(view) = view {
            self.halt_motion(view, &mut tick);
        }
        if self.active() {
            self.log(&mut tick, "stopped", None);
        }
        self.phase = Phase::Done;
        tick
    }

    /// Stop what the current step has in flight, keeping the rover's level.
    fn halt_motion(&self, view: &MissionView, tick: &mut MissionTick) {
        match self.phase {
            Phase::Docking { .. } => tick.commands.push(MissionCommand::CancelDock),
            Phase::Driving { .. } => self.halt_driving(view, tick),
            _ => {}
        }
    }

    fn halt_driving(&self, view: &MissionView, tick: &mut MissionTick) {
        if matches!(self.phase, Phase::Driving { .. }) {
            tick.commands.push(MissionCommand::Hold {
                x: view.x,
                y: view.y,
            });
        }
    }

    fn arrived(
        &mut self,
        step: &MissionStep,
        view: &MissionView,
        now: f64,
        tick: &mut MissionTick,
    ) {
        self.log(tick, "arrived", None);
        match step.kind {
            StepKind::Waypoint | StepKind::Collect => self.next(self.index + 1, view, now, tick),
            StepKind::Home => {
                self.phase = Phase::Waiting(Wait::Refill);
                self.log(tick, "waiting_refill", None);
            }
            StepKind::Score => self.start_dock(view, now, tick),
        }
    }

    fn start_dock(&mut self, _view: &MissionView, now: f64, tick: &mut MissionTick) {
        let Some(bucket) = self.current().and_then(|step| step.bucket) else {
            self.pause("Score step has no bucket".into(), tick);
            return;
        };
        self.resume_docking = true;
        tick.commands.push(MissionCommand::Dock { bucket });
        self.phase = Phase::Docking {
            sent_at: now,
            sends: 1,
            dock_id: None,
        };
        self.log(tick, "docking", None);
    }

    fn scored(&mut self, view: &MissionView, now: f64, tick: &mut MissionTick) {
        self.log(tick, "scored", None);
        self.next(self.index + 1, view, now, tick);
    }

    fn next(&mut self, to: usize, view: &MissionView, now: f64, tick: &mut MissionTick) {
        let to = if to >= self.plan.steps.len() {
            if self.plan.repeat {
                self.laps += 1;
                0
            } else {
                self.index = self.plan.steps.len();
                self.finish(tick);
                return;
            }
        } else {
            to
        };
        self.enter(to, view, now, tick);
    }

    fn enter(&mut self, index: usize, view: &MissionView, now: f64, tick: &mut MissionTick) {
        self.index = index;
        self.resume_docking = false;
        let Some(step) = self.current().cloned() else {
            self.finish(tick);
            return;
        };
        if step.kind == StepKind::Score && dock_window(&step, view) {
            self.log(tick, "started", None);
            self.start_dock(view, now, tick);
            return;
        }
        let target = goal_point(&step, view);
        tick.commands.push(MissionCommand::Goal {
            x: target.0,
            y: target.1,
            yaw: goal_yaw(&step, view, target),
        });
        self.phase = Phase::Driving {
            sent_at: now,
            sends: 1,
            seen: false,
            target,
        };
        self.log(tick, "started", None);
    }

    fn pause(&mut self, reason: String, tick: &mut MissionTick) {
        self.log(tick, "paused", Some(reason.clone()));
        self.phase = Phase::Paused(reason);
    }

    fn finish(&mut self, tick: &mut MissionTick) {
        self.index = self.index.min(self.plan.steps.len().saturating_sub(1));
        self.log(tick, "done", None);
        self.phase = Phase::Done;
    }

    fn log(&self, tick: &mut MissionTick, event: &'static str, detail: Option<String>) {
        let kind = self.current().map_or(StepKind::Waypoint, |step| step.kind);
        tick.events.push(MissionEvent {
            index: self.index,
            kind,
            event,
            detail,
        });
    }
}

/// Where a step's goal goes. A score step's goal sits [`SCORE_GOAL_M`] short
/// of the bucket's live position, on the line from the rover.
pub fn goal_point(step: &MissionStep, view: &MissionView) -> (f64, f64) {
    if step.kind != StepKind::Score {
        return (step.x, step.y);
    }
    let (bx, by) = bucket_position(step, view);
    let (dx, dy) = (bx - view.x, by - view.y);
    let distance = dx.hypot(dy);
    if distance <= SCORE_GOAL_M {
        return (view.x, view.y);
    }
    let scale = (distance - SCORE_GOAL_M) / distance;
    (view.x + dx * scale, view.y + dy * scale)
}

/// The rover is within [`DOCK_START_M`] of the step's bucket and facing it
/// within [`DOCK_START_BEARING_DEG`]: the dock can see the bucket from here.
pub fn dock_window(step: &MissionStep, view: &MissionView) -> bool {
    let (bx, by) = bucket_position(step, view);
    in_dock_window(view.x, view.y, view.yaw, bx, by)
}

/// [`dock_window`] for a rover at `(x, y)` heading `yaw` and a bucket at `(bx, by)`.
pub fn in_dock_window(x: f64, y: f64, yaw: f64, bx: f64, by: f64) -> bool {
    let (dx, dy) = (bx - x, by - y);
    if !(dx.is_finite() && dy.is_finite() && yaw.is_finite()) {
        return false;
    }
    let bearing = (dy.atan2(dx) - yaw + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
        - std::f64::consts::PI;
    dx.hypot(dy) <= DOCK_START_M && bearing.abs() <= DOCK_START_BEARING_DEG.to_radians()
}

fn bucket_position(step: &MissionStep, view: &MissionView) -> (f64, f64) {
    step.bucket
        .and_then(|id| view.buckets.iter().find(|(seen, _, _)| *seen == id))
        .map_or((step.x, step.y), |(_, x, y)| (*x, *y))
}

/// A score step arrives at `target` facing its bucket, so the camera can lock on it.
fn goal_yaw(step: &MissionStep, view: &MissionView, target: (f64, f64)) -> Option<f64> {
    if step.kind != StepKind::Score {
        return None;
    }
    let (bx, by) = bucket_position(step, view);
    let (dx, dy) = (bx - target.0, by - target.1);
    (dx.hypot(dy) > 1e-6).then(|| dy.atan2(dx))
}
