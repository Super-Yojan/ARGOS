//! Fleet picture and operator commands for the ARGOS map.
//!
//! The window subscribes to Zenoh and draws rover markers, the local occupancy
//! grid, and the planner path. Dragging a selected rover sends a goal on
//! release. Teleop uses Terra's `<id>/teleop` payload. Twist limits here are
//! defense in depth; Terra enforces the real limits.

mod fleet;
pub use fleet::*;
