//! Background Zenoh subscription and the commands the selected rover may receive.
//!
//! Connect and snapshot polling run on a thread named `argos-fleet`. Publishing
//! stays on that client and is idle until the window selects a rover. Only the
//! selected rover's occupancy grid, point cloud, and depth are subscribed; the
//! rest of the fleet sends pose and status only.
//!
//! | File | What it holds |
//! | --- | --- |
//! | `config.rs` | Defaults, `.env` loading, `DashboardConfig`, log and capture folders |
//! | `marks.rs` | Snapshot → `RoverMark` per rover, presence rows, camera framing |
//! | `feed.rs` | `FleetFeed` handle: start, latest picture, teleop limits |
//! | `commands.rs` | Every command sent to a rover |
//! | `poll.rs` | The `argos-fleet` thread: connect, reconnect, refresh every 100 ms |

mod commands;
mod config;
mod feed;
mod marks;
mod overlay;
mod poll;

pub use config::*;
pub use feed::*;
pub use marks::*;
pub use overlay::OverlayFrames;

#[cfg(test)]
#[path = "../../../../tests/argos-gpui/unit/fleet_tests.rs"]
mod tests;
