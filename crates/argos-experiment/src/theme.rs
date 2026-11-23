//! Colours and fonts of the operator console.
//!
//! Neutral carbon-dark operations palette: deep obsidian chrome, graphite
//! framed panes, elevated slate cards, crisp 1 px hairlines, and high-contrast
//! tactical cobalt / crimson / emerald accents.
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

/// Deep neutral carbon window backdrop.
pub const BG: u32 = 0x0b0c0e;
/// Primary framed pane surface (Rovers, Map, Focus, TopBar).
pub const PANEL: u32 = 0x141518;
/// Elevated card / recessed control track surface.
pub const PANEL_MUTED: u32 = 0x1c1e22;
/// Crisp dark charcoal canvas for the 2D fleet map.
pub const MAP_BG: u32 = 0x0f1114;
/// Subtle explored free space on the 2D map.
pub const MAP_FREE: u32 = 0x171b20;
/// Quiet 1 m reference grid on the 2D map.
pub const MAP_GRID: u32 = 0x1e2229;
/// High-contrast cool slate obstacle cells on the 2D map.
pub const MAP_OBSTACLE: u32 = 0x94a3b8;
/// Neutral dark backdrop for the 3D scene and FPV viewport.
pub const SCENE_BG: u32 = 0x0d0f12;
/// Crisp 1 px graphite hairline border.
pub const BORDER: u32 = 0x262930;
/// Whisper-quiet inner divider between pane headers and sections.
pub const BORDER_SUBTLE: u32 = 0x1e2026;

// -------------------------------------------------------------------- text

/// Crisp off-white primary text.
pub const TEXT: u32 = 0xf3f4f6;
/// Neutral slate secondary text.
pub const TEXT_MUTED: u32 = 0x8b909a;

// ----------------------------------------------------------------- accents

/// Selection, routes and the primary action (tactical cobalt).
pub const BLUE: u32 = 0x2563eb;
pub const BLUE_TEXT: u32 = 0x60a5fa;
pub const BLUE_PALE: u32 = 0xdbeafe;
pub const BLUE_BG: u32 = 0x172554;
/// Text on a filled blue button.
pub const INK: u32 = 0xffffff;

/// "All good" / active emerald.
pub const GREEN: u32 = 0x34d399;

/// The STOP buttons.
pub const STOP: u32 = 0xdc2626;
pub const STOP_TEXT: u32 = 0xffffff;

/// Rovers that need nothing from the operator, drawn small on the map.
pub const QUIET_ROVER: u32 = 0x64748b;
pub const QUIET_ROUTE: u32 = 0x475569;
pub const QUIET_LABEL: u32 = 0xcbd5e1;

/// The colours of one kind of alert: a card, its text and its buttons.
#[derive(Clone, Copy)]
pub struct Tone {
    /// Border, left accent bar, ring on the map, outline button.
    pub accent: u32,
    /// Filled button and badge.
    pub fill: u32,
    /// Text on the filled button.
    pub ink: u32,
    /// Text on the card.
    pub text: u32,
    /// Card background.
    pub bg: u32,
}

/// A rover that is stopped (incident style).
pub const RED: Tone = Tone {
    accent: 0xdc2626,
    fill: 0xdc2626,
    ink: 0xffffff,
    text: 0xfca5a5,
    bg: 0x1e1315,
};

/// A rover that waits for the operator.
pub const ORANGE: Tone = Tone {
    accent: 0xd97706,
    fill: 0xd97706,
    ink: 0xffffff,
    text: 0xfcd34d,
    bg: 0x1d1812,
};

/// Amber text on a dark panel (a count, a state).
pub const ORANGE_TEXT: u32 = 0xfbbf24;
