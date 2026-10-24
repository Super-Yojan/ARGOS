//! Console colors for the light field background. Text pairs keep at least
//! 4.5:1 contrast against `BACKGROUND`, `PANEL`, and `SURFACE`. Map overlays
//! and the video frame keep their own colors because they draw over imagery.

/// Window, sidebar, and theme background.
pub const BACKGROUND: u32 = 0xf1f4f3;
/// Side panels set off from the background, such as FPV.
pub const PANEL: u32 = 0xe8eeef;
/// Rover cards in the sidebar.
pub const SURFACE: u32 = 0xffffff;
pub const BORDER: u32 = 0xc9d3d6;
pub const TEXT: u32 = 0x17242c;
pub const MUTED: u32 = 0x4f6169;
/// Notices and drive blocks.
pub const WARNING: u32 = 0x8a5a00;
/// Heading hold, distinct from `WARNING`.
pub const HOLD: u32 = 0x8a4f00;
/// Card border for the selected rover.
pub const SELECTED: u32 = 0x1d8a64;
/// Card border for a rover that needs attention.
pub const ATTENTION: u32 = 0xb0661c;
/// Idle button fill.
pub const CONTROL: u32 = 0xdfe6e8;
/// Held, chosen, or go button fill.
pub const ACTIVE: u32 = 0xc4e6d5;
/// Stop button fill.
pub const DANGER: u32 = 0xf2d3d0;
/// FPV phase text.
pub const OK_TEXT: u32 = 0x157a57;
pub const ERROR_TEXT: u32 = 0xa83a28;
/// Fill for a risky mode that is switched on, such as in-place spin.
pub const CAUTION: u32 = 0xf3e1b8;
