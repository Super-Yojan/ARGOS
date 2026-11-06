//! Everything the dashboard window draws and does. `main.rs` holds the
//! `Dashboard` state and starts the window; this folder holds the rest.
//!
//! | File | What it holds |
//! | --- | --- |
//! | `state.rs` | Startup, the 100 ms sync from `FleetFeed`, 3D rover markers |
//! | `actions.rs` | What buttons and keys do: select, stop, drive, study, explore, task |
//! | `render.rs` | The window layout and its mouse and key handlers |
//! | `commands.rs` | Menu bar, keyboard shortcuts, and the actions they send |
//! | `console_bar.rs` | Top bar: map mode, grid, status, link, Stop all |
//! | `cards.rs` | Rover callout cards and off-screen tabs |
//! | `popups.rs` | Dump approval and click-to-goal confirm popups |
//! | `inspector.rs` | The selected rover's panel and its Overview tab |
//! | `levels.rs` | Autonomy level buttons and the study condition |
//! | `task.rs` | L4 Explore and L5 Task sections |
//! | `sections.rs` | Goal, bucket dock, and 3D capture sections |
//! | `mission.rs` | Mission tab: planned steps, the per-rover mission runner |
//! | `drive.rs` | Teleop section, turning limits, W/A/S/D pad |
//! | `video.rs` | FPV video tab |
//! | `widgets.rs` | Shared section titles, hints, cautions, rows |
//! | `agent.rs` | Agent bridge, recommendation cards, raw-drive grant |
//! | `interaction.rs` | Click-to-place goals and goal submission |
//! | `map_overlay.rs`, `local_map.rs` | Grid, route, and goal drawing on the 3D map |
//! | `palette.rs` | Console colors |
//! | `panels.rs` | Dock panels: Map center, Rover right, optional panels from the Panels menu |
//! | `timeline.rs` | Timeline panel: per-rover neglect, waiting, interaction, predicted need |
//! | `attention.rs` | Attention log: selection, video, window focus, card states, poses, gaze, blinks |
//!
//! Fleet data, `.env` settings, and sending commands live in the library
//! (`fleet/`, `console.rs`), not here.

mod actions;
pub(crate) mod agent;
pub(crate) mod attention;
mod cards;
pub(crate) mod commands;
mod console_bar;
mod drive;
mod inspector;
mod interaction;
mod levels;
mod local_map;
mod map_overlay;
pub(crate) mod mission;
pub(crate) mod palette;
pub(crate) mod panels;
mod popups;
mod render;
mod sections;
mod state;
mod task;
mod timeline;
mod video;
mod widgets;
