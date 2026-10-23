//! Fleet picture and operator commands for the ARGOS map.
//!
//! The window subscribes to Zenoh and draws rover markers, the local occupancy
//! grid, and the planner path. Dragging a selected rover sends a goal on
//! release. Teleop uses Terra's `<id>/teleop` payload. Twist limits here are
//! defense in depth; Terra enforces the real limits.

mod fleet;
pub use fleet::*;

mod launch;
pub use launch::{LAUNCH_HELP, LaunchMode};

mod occupancy;
pub use occupancy::occupancy_runs;

mod model_drag;
pub use model_drag::model_position;

mod fpv;
pub use fpv::{FpvConfig, FpvController, FpvPhase, FpvView, SenderOptions, run_test_sender};
