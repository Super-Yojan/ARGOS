//! Colours and fonts of the operator console.
//!
//! Colours are `0xRRGGBB` values; wrap them in `rgb(..)` where they are used.

// ------------------------------------------------------------------ fonts

/// Body text.
pub const FONT_UI: &str = "Inter";
/// Headings and the big buttons.
pub const FONT_HEADING: &str = "Space Grotesk";
/// Numbers, states and small labels.
pub const FONT_MONO: &str = "IBM Plex Mono";

// ---------------------------------------------------------------- surfaces

pub const BG: u32 = 0x0f1513;
pub const PANEL: u32 = 0x171f1c;
pub const MAP_BG: u32 = 0x131a18;
pub const MAP_FREE: u32 = 0x1e2c27;
pub const MAP_GRID: u32 = 0x24332e;
pub const MAP_OBSTACLE: u32 = 0x8aa399;
pub const SCENE_BG: u32 = 0x050807;
pub const BORDER: u32 = 0x2c3833;

// -------------------------------------------------------------------- text

pub const TEXT: u32 = 0xe4ebe8;
pub const TEXT_MUTED: u32 = 0x97a6a1;

// ----------------------------------------------------------------- accents

/// Selection, routes and the main action.
pub const BLUE: u32 = 0x7ea3ff;
pub const BLUE_TEXT: u32 = 0xa9c1ff;
pub const BLUE_PALE: u32 = 0xcfdcff;
pub const BLUE_BG: u32 = 0x1a2440;
/// Text on a blue button.
pub const INK: u32 = 0x0f1513;

/// "All good".
pub const GREEN: u32 = 0x6fd3a0;

/// The STOP buttons.
pub const STOP: u32 = 0xb3261e;
pub const STOP_TEXT: u32 = 0xffffff;

/// Rovers that need nothing from the operator, drawn small on the map.
pub const QUIET_ROVER: u32 = 0x7f97c9;
pub const QUIET_ROUTE: u32 = 0x56709f;
pub const QUIET_LABEL: u32 = 0xb9c8ea;

/// The colours of one kind of alert: a card, its text and its buttons.
#[derive(Clone, Copy)]
pub struct Tone {
    /// Border, ring on the map, outline button.
    pub accent: u32,
    /// Filled button.
    pub fill: u32,
    /// Text on the filled button.
    pub ink: u32,
    /// Text on the card.
    pub text: u32,
    /// Card background.
    pub bg: u32,
}

/// A rover that is stopped.
pub const RED: Tone = Tone {
    accent: 0xff8a80,
    fill: 0xffb4ab,
    ink: 0x2a0f0d,
    text: 0xffd9d4,
    bg: 0x3a1d1b,
};

/// A rover that waits for the operator.
pub const ORANGE: Tone = Tone {
    accent: 0xff9a5a,
    fill: 0xff9a5a,
    ink: 0x2a1608,
    text: 0xffe2cf,
    bg: 0x3a2416,
};

/// Orange text on a dark panel (a count, a state).
pub const ORANGE_TEXT: u32 = 0xffb685;
