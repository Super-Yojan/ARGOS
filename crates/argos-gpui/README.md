# GPUI dashboard code

The library receives fleet telemetry; the executable renders it and handles operator input.

| File | Responsibility |
| --- | --- |
| `src/main.rs` | Dashboard state, telemetry synchronization, control actions, and panel composition |
| `src/console.rs` | C3 console presentation: each rover's field callout, attention, inspector trace and sections, unlocated fleet members, and field notices |
| `src/fleet.rs` | Connection configuration, fleet observations, and outgoing commands |
| `src/interaction.rs` | Rover pickup, goal preview, release, cancellation, and navigation admission |
| `src/local_map.rs` | Local occupancy grid, routes, and grid coordinate conversion |
| `src/map_overlay.rs` | Geographic occupancy, routes, and destination overlays |
| `src/model_drag.rs` | Pure conversion from a drag destination to the preview model position |
| `src/occupancy.rs` | Lossless occupancy spans that reduce rendering work |
| `src/launch.rs` | Real/simulation launch selection |

Clicking a rover selects it and takes the window's **local** control claim. It does not arm the rover or acquire an exclusive remote lease. With a rover selected, a plain click on the main map proposes a destination there and opens a small popup at the click. **Send goal** submits it through the existing admission checks; **Cancel** or Escape sends nothing. The rover model stays at its reported position. Releasing control deselects the rover and closes its panel. The local map tab is display-only.

The `terra-map` crate owns the camera and tile renderer. A map drag only continues after a map press; pointer movement cannot acquire a gesture begun on a rover. Loaded tile triangles provide visual surface heights for model placement and map projection. The lookup includes roofs and canopies, so it is not a traversability map or navigation feedback. Without loaded surface data, placement falls back to the camera reference height.

Run library checks with `cargo test -p argos-gpui --lib` and build the UI with `cargo build -p argos-gpui`. Map tests live in `terra-map`; explicitly enabled GPU tests cover rendered surface heights and depth rendering.
