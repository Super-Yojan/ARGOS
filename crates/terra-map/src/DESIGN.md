# Map surface design

Mode: operate / experience. The 3D map occupies the entire viewport; instrument
panels float around the edges and preserve a large central navigation area.

Visual authority: the user's six-panel RAYDATA city-map reference, supplied
2026-10-19. Use its dark maritime palette, subdued urban geometry, translucent
instrument rails, fine circular navigation marks and precise cyan highlighting.

## Tokens

- Canvas: #0b141d; panel: #101f2bec; panel solid: #101f2b.
- Primary text: #e5f1f5; secondary text: #9bb4c2; cyan: #86dce8.
- Hairline: #2d4859. Active control: #193847.
- Typography: system sans for titles and guidance; Menlo for coordinates and
  numeric instruments. Body 14 px, labels 11 px, measured values 20–28 px.
- Panels: 1 px borders, 6 px corners. Controls: 4 px corners, visible hover/focus.
- Layout: 24 px desktop edge inset; 232 px left instrument panel; 96 px compass.
  Hide the large instrument rail below 900 px; retain map and core controls.

## Behavior

Survey mode desaturates imagery into a blue-green tonal range in the native GPU
shader. Imagery mode restores source colors. Both retain Google attribution.
Navigation controls adjust the actual camera; coordinates, range, heading,
pitch, tile count, pixel size and LOD threshold reflect current application state.

No decorative metrics, fake maps, looping motion, or analytics claims. Errors
and offline scenes must remain clearly labeled. A missing key displays setup
instructions in the viewport. Credits sit on a solid dark backing for contrast.

2D/3D is an independent control beside the compass, with keyboard shortcuts 2/3.
2D uses parallel projection, north-up orientation, viewport-scaled dragging,
and the existing tile imagery. Return to 3D restores heading and pitch.
The rendering rail exposes measured pipeline timings; label GPU wait/copy as
a combined wall-clock stage rather than claiming hardware GPU duration.
