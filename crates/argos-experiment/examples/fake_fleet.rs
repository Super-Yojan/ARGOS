//! A few pretend rovers, for working on the console without hardware.
//!
//! They speak the real `terra-comms` protocol, so the console cannot tell
//! them from Terra rovers: they drive to goals, obey the stick while no
//! task is running, and honour pause, resume, clear and the e-stop.
//! Rover 3 starts out stuck, so the help queue is never empty.
//!
//! Their world has one wall, at x = 6 m. Every rover has it in its
//! occupancy grid, and reports it while running the `remote` controller,
//! which is what the console's obstacle assist (L1) needs. The fake `dwa` does not steer around it.
//!
//!     cargo run -p terra-comms --example router      # in ../Terra
//!     cargo run -p argos-experiment --example fake_fleet
//!     cargo run -p argos-experiment

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use terra_comms::{
    Attention, Capabilities, CommsConfig, Goal, GoalKind, GpsAnchor, Occupancy, PlannedPath, Pose,
    Pose2, RemoteState, RoverLink, StageOption, Status, Task, TaskAction, TaskState, TaskStatus,
    Twist,
};

const TICK: Duration = Duration::from_millis(50);
const SPEED: f64 = 0.5; // m/s
const EARTH_RADIUS_M: f64 = 6_371_000.0;

struct FakeRover {
    link: RoverLink,
    x: f64,
    y: f64,
    yaw: f64,
    gps_anchor: Option<GpsAnchor>,
    task: Option<Task>,
    state: TaskState,
    started: f64,
}

impl FakeRover {
    fn connect(id: u64, x: f64, y: f64, gps_anchor: Option<GpsAnchor>) -> Self {
        let endpoint =
            std::env::var("TERRA_ENDPOINT").unwrap_or_else(|_| "tcp/127.0.0.1:7447".into());
        let config =
            format!("[comms]\nendpoint = \"{endpoint}\"\nprefix = \"terra\"\nrover_id = {id}\n");
        let link = CommsConfig::from_toml(&config)
            .and_then(|config| terra_comms::connect(&config))
            .and_then(|link| link.rover())
            .expect("fake rover config");
        link.publish(&capabilities());
        // The map never changes here; the link repeats it for late joiners.
        link.publish(&Occupancy {
            t: now(),
            resolution: 0.1,
            cells: wall()
                .iter()
                .map(|cell| [cell[0] as f32, cell[1] as f32])
                .collect(),
            free: Vec::new(),
        });
        Self {
            link,
            x,
            y,
            yaw: 0.0,
            gps_anchor,
            task: None,
            state: TaskState::Idle,
            started: now(),
        }
    }

    fn tick(&mut self, dt: f64) {
        let estopped = self.link.estop();

        if let Some(task) = self.link.take_task() {
            self.state = TaskState::Running {
                distance_to_goal: 0.0,
                elapsed: 0.0,
            };
            self.started = now();
            self.task = Some(task);
            self.publish_planned_path();
        }
        while let Some(command) = self.link.take_task_command() {
            if self
                .task
                .as_ref()
                .is_none_or(|task| task.id != command.task_id)
            {
                continue;
            }
            match command.action {
                TaskAction::Clear => {
                    (self.task, self.state) = (None, TaskState::Idle);
                    self.publish_planned_path();
                }
                TaskAction::Pause => self.state = TaskState::Paused,
                TaskAction::Resume if !estopped => {
                    self.state = TaskState::Running {
                        distance_to_goal: 0.0,
                        elapsed: 0.0,
                    };
                }
                TaskAction::Resume => {}
            }
        }

        let running = matches!(self.state, TaskState::Running { .. });
        let remote = running
            && self
                .task
                .as_ref()
                .is_some_and(|task| task.controller == "remote");
        if running && estopped {
            self.state = TaskState::NeedsAttention {
                reason: Attention::EStop,
                since: now(),
            };
        } else if remote {
            // The console steers; tell it what this rover sees.
            let command = self.link.remote_cmd().unwrap_or(terra_comms::Teleop {
                t: 0.0,
                v: 0.0,
                w: 0.0,
            });
            self.steer(command.v, command.w, dt);
            self.link.publish(&RemoteState {
                t: now(),
                pose: Pose2 {
                    x: self.x,
                    y: self.y,
                    yaw: self.yaw,
                },
                twist: Twist {
                    v: command.v,
                    w: command.w,
                },
                path: Vec::new(),
                obstacles: wall(),
            });
        } else if running {
            self.drive_to_goal(dt);
        } else if let (Some(stick), false) = (self.link.teleop(), estopped) {
            self.steer(stick.v, stick.w, dt);
        }

        self.link.publish(&Pose {
            t: now(),
            x: self.x,
            y: self.y,
            yaw: self.yaw,
            tracked: true,
        });
        self.link.publish(&Status {
            t: now(),
            estopped,
            link_ok: true,
            detail: String::new(),
            gps_anchor: self.gps_anchor.clone(),
            gate_open: false,
        });
        self.link.publish(&TaskStatus {
            t: now(),
            task_id: self.task.as_ref().map(|task| task.id),
            state: self.state.clone(),
            controller: self
                .task
                .as_ref()
                .map_or("teleop".into(), |task| task.controller.clone()),
            controller_version: "1".into(),
            params: Default::default(),
            goal: self
                .task
                .as_ref()
                .map_or(Goal::Stop, |task| task.goal.clone()),
            timeout: None,
            last_rejected: None,
        });
    }

    fn steer(&mut self, v: f64, w: f64, dt: f64) {
        self.yaw += w * dt;
        self.x += v * self.yaw.cos() * dt;
        self.y += v * self.yaw.sin() * dt;
    }

    fn goal_world(&self, goal: &Goal) -> Option<(f64, f64, f64)> {
        match goal {
            Goal::Pose {
                x, y, tolerance, ..
            } => Some((*x, *y, *tolerance)),
            Goal::Gps {
                latitude,
                longitude,
                tolerance,
                ..
            } => {
                let anchor = self.gps_anchor.as_ref()?;
                let d_lat = (latitude - anchor.latitude).to_radians();
                let d_lon = (longitude - anchor.longitude).to_radians();
                let north = EARTH_RADIUS_M * d_lat;
                let east = EARTH_RADIUS_M * anchor.latitude.to_radians().cos() * d_lon;
                let (sin_h, cos_h) = anchor.heading.sin_cos();
                let x = east * sin_h + north * cos_h;
                let y = -east * cos_h + north * sin_h;
                Some((x, y, *tolerance))
            }
            Goal::Stop | Goal::Explore { .. } => None,
        }
    }

    fn drive_to_goal(&mut self, dt: f64) {
        let Some(goal) = self.task.as_ref().map(|task| task.goal.clone()) else {
            return;
        };
        let Some((x, y, tolerance)) = self.goal_world(&goal) else {
            if matches!(goal, Goal::Gps { .. }) {
                self.state = TaskState::NeedsAttention {
                    reason: Attention::NoGpsAnchor,
                    since: now(),
                };
            }
            return;
        };
        let (dx, dy) = (x - self.x, y - self.y);
        let distance = dx.hypot(dy);
        if distance <= tolerance {
            self.state = TaskState::Reached;
            self.publish_planned_path();
            return;
        }
        self.yaw = dy.atan2(dx);
        self.x += dx / distance * SPEED * dt;
        self.y += dy / distance * SPEED * dt;
        self.state = TaskState::Running {
            distance_to_goal: distance,
            elapsed: now() - self.started,
        };
    }

    fn publish_planned_path(&self) {
        let poses = match (&self.task, &self.state) {
            (
                Some(Task {
                    goal,
                    planner: Some(_),
                    ..
                }),
                TaskState::Running { .. } | TaskState::Paused | TaskState::NeedsAttention { .. },
            ) => self
                .goal_world(goal)
                .map(|(x, y, _)| sample_path([self.x, self.y], [x, y]))
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        self.link.publish(&PlannedPath { t: now(), poses });
    }
}

/// Sample a simple planned route between `start` and `goal` for visualisation.
fn sample_path(start: [f64; 2], goal: [f64; 2]) -> Vec<Pose2> {
    let steps = 16usize;
    let dx = goal[0] - start[0];
    let dy = goal[1] - start[1];
    let yaw = dy.atan2(dx);
    (0..=steps)
        .map(|i| {
            let t = i as f64 / steps as f64;
            Pose2 {
                x: start[0] + dx * t,
                y: start[1] + dy * t,
                yaw,
            }
        })
        .collect()
}

fn capabilities() -> Capabilities {
    let stage = |id: &str, description: &str, goals: Vec<GoalKind>| StageOption {
        id: id.into(),
        version: "1".into(),
        description: description.into(),
        params: Vec::new(),
        goals,
    };
    let takes_goals = vec![GoalKind::Local, GoalKind::Gps];
    Capabilities {
        t: now(),
        controllers: vec![
            stage("teleop", "Drive from a stick", vec![]),
            stage(
                "drive_pose",
                "Steer onto the next path pose (no obstacle avoidance)",
                takes_goals.clone(),
            ),
            stage(
                "dwa",
                "Dynamic window approach: follows the path and avoids obstacles",
                takes_goals.clone(),
            ),
            stage(
                "remote",
                "Twist from the base station on remote/cmd",
                takes_goals.clone(),
            ),
        ],
        planners: vec![
            stage("straight_line", "Direct line to goal", takes_goals.clone()),
            stage("hybrid_astar", "Obstacle-avoiding global path", takes_goals),
        ],
        // Like the real rover: nothing drives until a task names a controller.
        default_controller: "teleop".into(),
        software: "fake".into(),
        footprint: Some([0.6, 0.45]),
    }
}

/// The wall every fake rover sees: x = 6 m, from y = -3 to 3, one point
/// every 10 cm.
fn wall() -> Vec<[f64; 2]> {
    (-30..=30).map(|step| [6.0, step as f64 * 0.1]).collect()
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64())
}

fn main() {
    let with_gps = std::env::args().any(|arg| arg == "--gps");
    let anchor = with_gps.then_some(GpsAnchor {
        latitude: 37.7749,
        longitude: -122.4194,
        heading: 0.0,
        accuracy: 1.5,
    });
    let mut rovers = vec![
        FakeRover::connect(1, -3.0, 2.0, anchor.clone()),
        FakeRover::connect(2, 2.0, 1.0, anchor.clone()),
        FakeRover::connect(3, 0.0, -2.5, anchor),
        // Leave rover 4 without a GPS anchor even in --gps mode so mixed fleets are easy to test.
        FakeRover::connect(4, 4.0, -1.0, None),
    ];

    // Rover 1 is on its way somewhere; rover 3 got stuck on its way.
    let goal = |x, y| Goal::Pose {
        x,
        y,
        yaw: None,
        tolerance: 0.25,
    };
    let task = |id, goal| Task {
        id,
        goal,
        controller: "dwa".into(),
        params: Default::default(),
        planner: Some("hybrid_astar".into()),
        planner_params: Default::default(),
    };
    rovers[0].task = Some(task(1, goal(3.0, 4.0)));
    rovers[0].state = TaskState::Running {
        distance_to_goal: 0.0,
        elapsed: 0.0,
    };
    rovers[0].publish_planned_path();
    rovers[2].task = Some(task(1, goal(-4.0, -4.0)));
    rovers[2].state = TaskState::NeedsAttention {
        reason: Attention::Stuck,
        since: now(),
    };
    rovers[2].publish_planned_path();

    println!("fake fleet: {} rovers, Ctrl-C to stop", rovers.len());
    loop {
        for rover in &mut rovers {
            rover.tick(TICK.as_secs_f64());
        }
        std::thread::sleep(TICK);
    }
}
