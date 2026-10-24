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
