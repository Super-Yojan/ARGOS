//! The console's one connection to the rovers.
//!
//! [`Fleet`] is the only state the components share. It wraps Terra's
//! `BaseLink` and does two things:
//!
//! - **reads**: [`Fleet::rovers`] turns each rover's latest messages into a
//!   [`Rover`];
//! - **writes**: [`Fleet::perform`], [`Fleet::go_to`], [`Fleet::drive`],
//!   [`Fleet::drive_assisted`] and [`Fleet::stop_all`] send commands.
//!
//! How autonomous a rover is (its [`Level`]) decides which controller and
//! planner those commands ask for; see [`level`].
//!
//! Commands are fire-and-forget. Whether one worked shows up in the next
//! `TaskStatus` / `Status` from the rover, so the UI never assumes it did.
//! The rover's own safety chain decides what is allowed; nothing is checked
//! twice here.
//!
//! Cloning a `Fleet` is cheap: every clone is the same connection.

pub mod assist;
pub mod level;
mod rover;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use terra_comms::{
    BaseLink, Capabilities, CaptureWorldMap, CommsConfig, Depth, Estop, GateCommand, Goal,
    LoadWorldMap, Occupancy, Params, PlannedPath, Pose, RemoteCmd, RemoteState, Status,
    Subscription, Task, TaskAction, TaskCommand, TaskStatus, Teleop, WorldMap, WorldMapRequest,
};

pub use level::Level;
pub use rover::{Activity, Request, Rover, RoverAction, Urgency, help_queue};
use rover::Reports;

/// How close counts as "arrived" for a goal clicked on the map, metres.
const GOAL_TOLERANCE: f64 = 0.25;

#[derive(Clone)]
pub struct Fleet {
    inner: Rc<Inner>,
}

/// Latest saved `ARWorldMap` archive shared across the fleet so multiple
/// rovers can relocalize into one world coordinate origin.
#[derive(Clone)]
struct SharedWorldMap {
    source_rover: Option<u64>,
    sequence: u64,
    data: Vec<u8>,
}

fn shared_world_map_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/argos/shared_world_map.arworldmap"))
}

/// Minimum size/feature count for a saved `ARWorldMap` to be relocalizable.
/// Sparse maps (e.g. captured before scanning the room) trap joining rovers in
/// ARKit's `.limited(.relocalizing)` state forever.
const MIN_SHARED_MAP_BYTES: usize = 100_000;
const MIN_SHARED_MAP_POINTS: usize = 100;

fn save_world_map_to_disk(data: &[u8]) {
    let Some(path) = shared_world_map_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(err) = std::fs::write(&path, data) {
        eprintln!("[WorldMap] failed to save {}: {err}", path.display());
    }
}

struct Inner {
    endpoint: String,
    /// `None` when the config was bad; see `config_error`.
    base: Option<Arc<BaseLink>>,
    config_error: Option<String>,
    /// Rovers this console has an e-stop engaged on. The link re-sends an
    /// engaged e-stop every second until it is released from here.
    stopped_here: RefCell<BTreeSet<u64>>,
    /// Per rover: what it needs the operator for, and since when.
    waiting: RefCell<BTreeMap<u64, (Activity, Instant)>>,
    /// Per rover: the level the operator picked for its goals.
    goal_levels: RefCell<BTreeMap<u64, Level>>,
    /// Per rover: operator-chosen controller overrides for each autonomy level.
    level_controllers: RefCell<BTreeMap<u64, BTreeMap<Level, String>>>,
    /// Shared `ARWorldMap` cached from the latest rover upload (or disk).
    shared_world_map: Arc<Mutex<Option<SharedWorldMap>>>,
    /// Keeps the `WorldMap` and `WorldMapRequest` subscriptions alive.
    _world_map_subs: Vec<Subscription>,
}

pub enum LinkState {
    Connected,
    Connecting,
    /// The config is wrong; retrying will not help.
    ConfigError(String),
}

impl Fleet {
    /// Connect using `TERRA_ENDPOINT` (default `tcp/127.0.0.1:7447`),
    /// `TERRA_PREFIX` (default `terra`), `TERRA_USER` and `TERRA_PASSWORD`.
    /// Returns at once; the link keeps retrying in the background.
    pub fn connect_from_env() -> Self {
        let env = |key: &str| std::env::var(key).ok().filter(|value| !value.is_empty());
        let endpoint = env("TERRA_ENDPOINT").unwrap_or_else(|| "tcp/127.0.0.1:7447".into());
        let prefix = env("TERRA_PREFIX").unwrap_or_else(|| "terra".into());

        let mut toml = format!("[comms]\nendpoint = \"{endpoint}\"\nprefix = \"{prefix}\"\n");
        if let Some(user) = env("TERRA_USER") {
            toml.push_str(&format!("user = \"{user}\"\n"));
        }

        let link = CommsConfig::from_toml(&toml)
            .map(|config| config.with_password(env("TERRA_PASSWORD")))
            .and_then(|config| terra_comms::connect(&config));
        let (base, config_error) = match link {
            Ok(link) => (Some(Arc::new(link.base())), None),
            Err(error) => (None, Some(error.to_string())),
        };

        // Start each ARGOS session with no cached shared map so moving to a new
        // room never traps rovers trying to relocalize against a previous venue.
        let shared_world_map: Arc<Mutex<Option<SharedWorldMap>>> = Arc::new(Mutex::new(None));
        let mut world_map_subs = Vec::new();
        if let Some(base) = &base {
            // Cache every relocalizable ARWorldMap uploaded by any rover and persist it.
            let cache = shared_world_map.clone();
            world_map_subs.push(base.subscribe::<WorldMap>(move |rover_id, map| {
                if map.data.len() < MIN_SHARED_MAP_BYTES
                    || map.points.len() < MIN_SHARED_MAP_POINTS
                {
                    eprintln!(
                        "[WorldMap] ignoring sparse map from Rover {rover_id} ({} pts, {} bytes) — scan more of the room first",
                        map.points.len(),
                        map.data.len(),
                    );
                    return;
                }
                eprintln!(
                    "[WorldMap] cached shared map from Rover {rover_id} (seq={}, {} pts, {:.2} MB)",
                    map.sequence,
                    map.points.len(),
                    map.data.len() as f64 / (1024.0 * 1024.0),
                );
                save_world_map_to_disk(&map.data);
                if let Ok(mut guard) = cache.lock() {
                    let seq = guard.as_ref().map_or(1, |prev| prev.sequence + 1);
                    *guard = Some(SharedWorldMap {
                        source_rover: Some(rover_id),
                        sequence: seq,
                        data: map.data,
                    });
                }
            }));

            // Automatically answer any rover's WorldMapRequest with the saved map.
            let cache = shared_world_map.clone();
            let base_reply = base.clone();
            world_map_subs.push(base.subscribe::<WorldMapRequest>(move |rover_id, req| {
                let saved = cache.lock().ok().and_then(|g| g.clone());
                match saved {
                    Some(map) if req.auto && map.source_rover == Some(rover_id) => {
                        eprintln!(
                            "[WorldMap] skipping auto-sync for Rover {rover_id} (it captured the current map)"
                        );
                    }
                    Some(map) => {
                        eprintln!(
                            "[WorldMap] serving saved map to Rover {rover_id} ({:.2} MB, auto={}, source={:?})",
                            map.data.len() as f64 / (1024.0 * 1024.0),
                            req.auto,
                            map.source_rover,
                        );
                        base_reply.send(
                            rover_id,
                            &LoadWorldMap { t: now(), sequence: map.sequence, data: map.data },
                        );
                    }
                    None => {
                        eprintln!(
                            "[WorldMap] Rover {rover_id} requested shared map, but none is saved yet"
                        );
                    }
                }
            }));
        }

        Self {
            inner: Rc::new(Inner {
                endpoint,
                base,
                config_error,
                stopped_here: RefCell::default(),
                waiting: RefCell::default(),
                goal_levels: RefCell::default(),
                level_controllers: RefCell::default(),
                shared_world_map,
                _world_map_subs: world_map_subs,
            }),
        }
    }

    // ------------------------------------------------------------- reading

    pub fn endpoint(&self) -> &str {
        &self.inner.endpoint
    }

    pub fn link_state(&self) -> LinkState {
        match (&self.inner.config_error, &self.inner.base) {
            (Some(error), _) => LinkState::ConfigError(error.clone()),
            (None, Some(base)) if base.connected() => LinkState::Connected,
            _ => LinkState::Connecting,
        }
    }

    /// The raw link, for components that subscribe to a stream (the mesh).
    pub fn base(&self) -> Option<&BaseLink> {
        self.inner.base.as_deref()
    }

    /// A shared handle to the raw link, for background workers (FPV WebRTC).
    pub fn base_arc(&self) -> Option<Arc<BaseLink>> {
        self.inner.base.clone()
    }

    /// Every rover heard from in the last few seconds, sorted by id.
    pub fn rovers(&self) -> Vec<Rover> {
        let Some(base) = &self.inner.base else {
            return Vec::new();
        };
        let stopped_here = self.inner.stopped_here.borrow();
        let goal_levels = self.inner.goal_levels.borrow();
        let level_controllers = self.inner.level_controllers.borrow();
        let empty = BTreeMap::new();
        let mut rovers: Vec<Rover> = base
            .rovers()
            .into_iter()
            .map(|id| {
                let reports = Reports {
                    pose: latest::<Pose>(base, id),
                    status: latest::<Status>(base, id),
                    task: latest::<TaskStatus>(base, id),
                    capabilities: latest::<Capabilities>(base, id),
                };
                let overrides = level_controllers.get(&id).unwrap_or(&empty);
                Rover::interpret(
                    id,
                    reports,
                    stopped_here.contains(&id),
                    goal_levels.get(&id).copied(),
                    overrides,
                )
            })
            .collect();
        self.time_the_waiting(&mut rovers);
        rovers
    }

    pub fn rover(&self, id: u64) -> Option<Rover> {
        self.rovers().into_iter().find(|rover| rover.id == id)
    }

    /// The rover's obstacle map: the occupied cells of its occupancy grid.
    /// `None` until the rover has sent one (older rover software never does).
    pub fn occupancy(&self, rover_id: u64) -> Option<Occupancy> {
        latest::<Occupancy>(self.inner.base.as_ref()?, rover_id)
    }

    /// The rover's current global-planner route in its world frame.
    /// `None` when the rover has not sent one, or when its route is empty.
    pub fn planned_path(&self, rover_id: u64) -> Option<PlannedPath> {
        latest::<PlannedPath>(self.inner.base.as_ref()?, rover_id)
            .filter(|path| !path.poses.is_empty())
    }

    /// The rover's latest 3D depth point cloud (camera frame, metres).
    pub fn depth(&self, rover_id: u64) -> Option<Depth> {
        latest::<Depth>(self.inner.base.as_ref()?, rover_id)
    }

    /// The rover's latest `ARWorldMap` (including its 3D feature point cloud
    /// in the world frame, metres).
    pub fn world_map(&self, rover_id: u64) -> Option<WorldMap> {
        latest::<WorldMap>(self.inner.base.as_ref()?, rover_id)
    }

    /// The rover's latest status report.
    pub fn status(&self, rover_id: u64) -> Option<Status> {
        latest::<Status>(self.inner.base.as_ref()?, rover_id)
    }

    /// What a rover at level L1 sees: its pose and the obstacles it has
    /// mapped, with the age of that report in seconds. `None` unless the
    /// rover is running the obstacle assist.
    pub fn remote_state(&self, rover_id: u64) -> Option<(RemoteState, f64)> {
        self.inner.base.as_ref()?.latest::<RemoteState>(rover_id)
    }

    /// Fill in `Rover::waiting`: how long each rover has needed the operator
    /// for its current reason. The clock restarts when the reason changes.
    fn time_the_waiting(&self, rovers: &mut [Rover]) {
        let mut waiting = self.inner.waiting.borrow_mut();
        waiting.retain(|id, _| rovers.iter().any(|rover| rover.id == *id));
        for rover in rovers {
            if rover.urgency().is_none() {
                waiting.remove(&rover.id);
                continue;
            }
            let entry = waiting
                .entry(rover.id)
                .and_modify(|(activity, since)| {
                    if *activity != rover.activity {
                        *activity = rover.activity.clone();
                        *since = Instant::now();
                    }
                })
                .or_insert_with(|| (rover.activity.clone(), Instant::now()));
            rover.waiting = entry.1.elapsed();
        }
    }

    // ------------------------------------------------------------- writing

    /// Send the command behind `action` to one rover. Actions that only
    /// change the console (`Focus`, `TakeOver`, `SetGoal`) send nothing.
    pub fn perform(&self, rover_id: u64, action: RoverAction) {
        match action {
            RoverAction::Resume => self.task_command(rover_id, TaskAction::Resume),
            RoverAction::Pause => self.task_command(rover_id, TaskAction::Pause),
            RoverAction::Clear => self.task_command(rover_id, TaskAction::Clear),
            RoverAction::Stop => self.set_stop(rover_id, true),
            RoverAction::ReleaseStop => self.set_stop(rover_id, false),
            RoverAction::SetLevel(Level::Explore) => self.start_explore(rover_id),
            RoverAction::SetLevel(level) => self.set_goal_level(rover_id, level),
            RoverAction::SetLevelController { level, controller } => {
                self.set_level_controller(rover_id, level, controller)
            }
            RoverAction::SetGate(open) => self.set_gate(rover_id, open),
            RoverAction::SyncWorldMap => {
                self.sync_world_map(rover_id);
            }
            RoverAction::CaptureWorldMap => {
                self.capture_world_map(rover_id);
            }
            RoverAction::Focus | RoverAction::TakeOver | RoverAction::SetGoal => {}
        }
    }

    /// Send the rover to a point in its world frame (metres), at its goal
    /// level. This replaces whatever task it had.
    pub fn go_to(&self, rover_id: u64, x: f64, y: f64) {
        let Some(rover) = self.rover(rover_id) else { return };
        let goal = Goal::Pose { x, y, yaw: None, tolerance: GOAL_TOLERANCE };
        self.send_task(rover_id, new_task_id(), goal, rover.goal_level);
    }

    /// Drive the rover's goals at `level` from now on. If it is on its way
    /// to a goal, it switches at once and keeps the goal. Levels that take
    /// no goals are ignored: the drive view handles those.
    fn set_goal_level(&self, rover_id: u64, level: Level) {
        if !level.takes_goals() {
            return;
        }
        self.inner.goal_levels.borrow_mut().insert(rover_id, level);

        // Same task id + same goal = "keep going, but drive it this way".
        let current = self.inner.base.as_ref().and_then(|base| latest::<TaskStatus>(base, rover_id));
        let overrides = self
            .inner
            .level_controllers
            .borrow()
            .get(&rover_id)
            .cloned()
            .unwrap_or_default();
        if let Some(TaskStatus { task_id: Some(task_id), goal: goal @ Goal::Pose { .. }, controller, .. }) = current
            && Level::of_controller_with(&controller, true, &overrides, Some(level)).takes_goals()
        {
            self.send_task(rover_id, task_id, goal, level);
        }
    }

    /// Choose which controller to use for `level` on `rover_id`. If the rover
    /// is currently running at that level, switch it immediately.
    fn set_level_controller(&self, rover_id: u64, level: Level, controller: String) {
        if !Level::CONFIGURABLE.contains(&level) {
            return;
        }
        let active_level = self.rover(rover_id).map(|rover| rover.level);
        {
            let mut map = self.inner.level_controllers.borrow_mut();
            let entry = map.entry(rover_id).or_default();
            if level.default_controller() == Some(controller.as_str()) {
                entry.remove(&level);
            } else {
                entry.insert(level, controller);
            }
        }

        if active_level != Some(level) {
            return;
        }
        if level == Level::Assisted {
            self.start_assist(rover_id);
        } else if level.takes_goals() {
            let current = self
                .inner
                .base
                .as_ref()
                .and_then(|base| latest::<TaskStatus>(base, rover_id));
            if let Some(TaskStatus {
                task_id: Some(task_id),
                goal: goal @ Goal::Pose { .. },
                ..
            }) = current
            {
                self.send_task(rover_id, task_id, goal, level);
            }
        }
    }

    /// Start autonomous frontier exploration (level L4) on a rover.
    pub fn start_explore(&self, rover_id: u64) {
        self.send_task(
            rover_id,
            new_task_id(),
            Goal::Explore { budget_s: None },
            Level::Explore,
        );
    }

    /// Open (`true`) or close (`false`) the rover's dump gate servo.
    pub fn set_gate(&self, rover_id: u64, open: bool) {
        if let Some(base) = &self.inner.base {
            base.send(rover_id, &GateCommand { t: now(), open });
        }
    }

    /// Push the base station's saved shared `ARWorldMap` to `rover_id` so it
    /// relocalizes into the fleet's shared world coordinate origin.
    pub fn sync_world_map(&self, rover_id: u64) -> bool {
        let Some(base) = &self.inner.base else { return false };
        let saved = self.inner.shared_world_map.lock().ok().and_then(|g| g.clone());
        let Some(map) = saved else {
            eprintln!("[WorldMap] cannot sync Rover {rover_id}: no shared map saved yet");
            return false;
        };
        eprintln!(
            "[WorldMap] pushing shared map to Rover {rover_id} ({:.2} MB)",
            map.data.len() as f64 / (1024.0 * 1024.0),
        );
        base.send(rover_id, &LoadWorldMap { t: now(), sequence: map.sequence, data: map.data });
        true
    }

    /// Ask `rover_id` to capture its current `ARWorldMap` and upload it as the
    /// fleet's shared map.
    pub fn capture_world_map(&self, rover_id: u64) {
        if let Some(base) = &self.inner.base {
            eprintln!("[WorldMap] requesting world map capture from Rover {rover_id}");
            base.send(rover_id, &CaptureWorldMap { t: now() });
        }
    }

    /// Start the obstacle assist (level L1) on a rover. The rover then
    /// reports what it sees ([`Self::remote_state`]) and expects a command
    /// from [`Self::drive_assisted`] every tick. This replaces its task.
    pub fn start_assist(&self, rover_id: u64) {
        self.send_task(rover_id, new_task_id(), Goal::Stop, Level::Assisted);
    }

    /// Send a task that asks for the controller and planner of `level`.
    fn send_task(&self, rover_id: u64, task_id: u64, goal: Goal, level: Level) {
        let overrides = self
            .inner
            .level_controllers
            .borrow()
            .get(&rover_id)
            .cloned()
            .unwrap_or_default();
        let (Some(base), Some(recipe)) = (&self.inner.base, level.recipe_with(&overrides)) else {
            return;
        };
        let task = Task {
            id: task_id,
            goal,
            controller: recipe.controller,
            // Empty: the rover fills in each parameter's default.
            params: Params::new(),
            planner: recipe.planner.map(Into::into),
            planner_params: Params::new(),
        };
        base.send(rover_id, &task);
    }

    /// One stick sample: forward speed (m/s) and turn rate (rad/s, CCW +).
    /// The rover forgets it after `terra_comms::COMMAND_LEASE`, so keep
    /// calling this at 20 Hz while driving.
    pub fn drive(&self, rover_id: u64, v: f64, w: f64) {
        if let Some(base) = &self.inner.base {
            base.send(rover_id, &Teleop { t: now(), v, w });
        }
    }

    /// One stick sample for a rover running the obstacle assist. Pass the
    /// command through [`assist::filter`] first. The rover gives up on the
    /// assist if these stop coming, so send one every tick, zeros included.
    pub fn drive_assisted(&self, rover_id: u64, v: f64, w: f64) {
        if let Some(base) = &self.inner.base {
            base.send(rover_id, &RemoteCmd(Teleop { t: now(), v, w }));
        }
    }

    /// E-stop every rover in sight.
    pub fn stop_all(&self) {
        let Some(base) = &self.inner.base else { return };
        for id in base.rovers() {
            self.set_stop(id, true);
        }
    }

    fn set_stop(&self, rover_id: u64, engaged: bool) {
        let Some(base) = &self.inner.base else { return };
        base.send(rover_id, &Estop { t: now(), engaged });
        let mut stopped_here = self.inner.stopped_here.borrow_mut();
        if engaged {
            stopped_here.insert(rover_id);
        } else {
            stopped_here.remove(&rover_id);
        }
    }

    /// A task command must name the rover's current task, or it is refused.
    fn task_command(&self, rover_id: u64, action: TaskAction) {
        let Some(base) = &self.inner.base else { return };
        let current_task = latest::<TaskStatus>(base, rover_id).and_then(|status| status.task_id);
        if let Some(task_id) = current_task {
            base.send(rover_id, &TaskCommand { task_id, action });
        }
    }
}

/// The newest message of one kind from a rover, without its age.
fn latest<M: terra_comms::Uplink>(base: &BaseLink, rover_id: u64) -> Option<M> {
    base.latest::<M>(rover_id).map(|(message, _age)| message)
}

/// This side's clock in seconds; every message carries one.
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since_epoch| since_epoch.as_secs_f64())
        .unwrap_or(0.0)
}

/// Task ids only have to be unique per rover; the clock in milliseconds is.
fn new_task_id() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since_epoch| since_epoch.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Wait until `check` holds for the rover, or fail after a few seconds.
    fn wait_for(fleet: &Fleet, rover_id: u64, what: &str, check: impl Fn(&Rover) -> bool) {
        for _ in 0..50 {
            if fleet.rover(rover_id).is_some_and(|rover| check(&rover)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("rover {rover_id} never became {what}: {:?}", fleet.rover(rover_id));
    }

    /// Start a router and `--example fake_fleet` first (see that example),
    /// then run `cargo test -p argos-experiment -- --ignored commands`.
    #[test]
    #[ignore = "needs a router and the fake_fleet example running"]
    fn commands_reach_the_rovers_and_their_answers_are_read_back() {
        let fleet = Fleet::connect_from_env();
        wait_for(&fleet, 2, "visible", |_| true);

        // The rover's occupancy grid arrives without asking, at any level.
        wait_for(&fleet, 2, "sending its map", |_| fleet.occupancy(2).is_some());
        let map = fleet.occupancy(2).unwrap();
        assert!(map.resolution > 0.0 && !map.cells.is_empty(), "an empty map: {map:?}");

        // A goal is driven with obstacle avoidance unless told otherwise,
        // and the level can be changed on the way without losing the goal.
        fleet.go_to(2, -6.0, 1.0);
        wait_for(&fleet, 2, "driving at L3", |rover| {
            matches!(rover.activity, Activity::Driving { .. }) && rover.level == Level::Planned
        });
        let task_id = fleet.rover(2).and_then(|rover| rover.task_id);
        fleet.perform(2, RoverAction::SetLevel(Level::Direct));
        wait_for(&fleet, 2, "driving at L2, same task", |rover| {
            rover.level == Level::Direct && rover.task_id == task_id && rover.goal == Some([-6.0, 1.0])
        });

        fleet.go_to(2, 3.0, 2.0);
        wait_for(&fleet, 2, "driving to the new goal", |rover| {
            matches!(rover.activity, Activity::Driving { .. }) && rover.goal == Some([3.0, 2.0])
        });
        fleet.perform(2, RoverAction::Pause);
        wait_for(&fleet, 2, "paused", |rover| rover.activity == Activity::Paused);
        fleet.perform(2, RoverAction::Resume);
        wait_for(&fleet, 2, "arrived", |rover| rover.activity == Activity::Arrived);
        fleet.perform(2, RoverAction::Clear);
        wait_for(&fleet, 2, "idle", |rover| rover.activity == Activity::Idle);

        let start = fleet.rover(4).and_then(|rover| rover.pose).expect("rover 4 has a pose");
        for _ in 0..10 {
            fleet.drive(4, -0.5, 0.0);
            std::thread::sleep(Duration::from_millis(50));
        }
        wait_for(&fleet, 4, "moved by the stick", |rover| {
            rover.pose.as_ref().is_some_and(|pose| pose.x < start.x - 0.1)
        });

        // L1: full speed at the wall (x = 6 m). The assist must stop the
        // rover short of it, and still let it back away.
        fleet.start_assist(4);
        wait_for(&fleet, 4, "assisted", |rover| rover.level == Level::Assisted && fleet.remote_state(4).is_some());
        let mut verdicts = Vec::new();
        for _ in 0..200 {
            let (state, _age) = fleet.remote_state(4).expect("the rover reports what it sees");
            let (v, w, verdict) = assist::filter(0.5, 0.0, &state);
            fleet.drive_assisted(4, v, w);
            verdicts.push(verdict);
            if verdict == assist::Verdict::Blocked {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(verdicts.last(), Some(&assist::Verdict::Blocked), "never reached the wall");
        assert!(verdicts.iter().any(|verdict| matches!(verdict, assist::Verdict::Slowed(_))), "never slowed down");
        std::thread::sleep(Duration::from_millis(300));
        let (stopped_at, _age) = fleet.remote_state(4).unwrap();
        assert!(stopped_at.pose.x < 5.6, "stopped too close to the wall: x = {}", stopped_at.pose.x);
        assert_eq!(assist::filter(-0.5, 0.0, &stopped_at).2, assist::Verdict::Clear, "cannot back away");
        fleet.perform(4, RoverAction::Clear);
        wait_for(&fleet, 4, "manual again", |rover| rover.level == Level::Manual);

        // Ask the rover itself, not the console's own bookkeeping.
        let rover_says_stopped = |fleet: &Fleet| {
            let status = fleet.base().and_then(|base| base.latest::<Status>(4));
            status.is_some_and(|(status, _age)| status.estopped)
        };
        fleet.perform(4, RoverAction::Stop);
        wait_for(&fleet, 4, "stopped", |rover| rover.activity == Activity::Stopped && rover_says_stopped(&fleet));
        fleet.perform(4, RoverAction::ReleaseStop);
        wait_for(&fleet, 4, "released", |rover| rover.activity == Activity::Idle && !rover_says_stopped(&fleet));
    }
}
