//! Console colors for the light field background. Text colors keep at least
//! 4.5:1 contrast against `BACKGROUND`, and white text keeps it on the
//! status fills.

/// Window, title bar, and theme background.
pub const BACKGROUND: u32 = 0xf1f4f3;
pub const TEXT: u32 = 0x17242c;
pub const MUTED: u32 = 0x4f6169;
/// Status colors, used both as text on the background and as button fills.
pub const WARNING: u32 = 0x8a5a00;
pub const SUCCESS: u32 = 0x157a57;
pub const DANGER: u32 = 0xa83a28;
/// Picked toggles and focus. White text on it is 6.7:1.
pub const ACCENT: u32 = 0x1d4ed8;

// Map overlay colors. They sit on true-colour satellite imagery, so each
// mark is saturated and drawn over a dark halo (`MAP_INK`).

/// Halo, outlines, and badge fills under map marks.
pub const MAP_INK: u32 = 0x0f172a;
/// Badge text and the running-step ring.
pub const MAP_PAPER: u32 = 0xffffff;
/// Occupancy: cells the rover has seen clear.
pub const MAP_FREE: u32 = 0x22c55e;
/// Occupancy: cells holding an obstacle.
pub const MAP_OBSTACLE: u32 = 0xef4444;
/// Occupancy: never seen. Not painted; listed so the legend can name it.
pub const MAP_UNKNOWN: u32 = 0x94a3b8;
/// Planned route, driving forward.
pub const MAP_ROUTE: u32 = 0x38bdf8;
/// Route segments driven in reverse, and the drag-to-goal ghost.
pub const MAP_REVERSE: u32 = 0xfbbf24;
/// Destination ghost, arrival ring, and drag line.
pub const MAP_GOAL: u32 = 0xfbbf24;
/// Rover handle when this window cannot drag it.
pub const MAP_ROVER: u32 = 0x475569;
/// Line joining mission steps.
pub const MAP_MISSION: u32 = 0xf8fafc;
