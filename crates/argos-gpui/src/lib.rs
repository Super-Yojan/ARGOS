//! Read-only fleet picture for the ARGOS map.
//!
//! The window subscribes to Zenoh and places rover markers. It does not publish
//! goals, teleop, or any other command.

mod fleet;
pub use fleet::*;
