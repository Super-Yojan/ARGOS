# GPUI dashboard code

The library receives fleet telemetry; the executable renders it and handles operator input.

| File | Responsibility |
| --- | --- |
| `src/main.rs` | Dashboard state, telemetry synchronization, control actions, and panel composition |
| `src/console.rs` | C3 console presentation: each rover's field callout, attention, and inspector trace |
| `src/fleet.rs` | Connection configuration, fleet observations, and outgoing commands |
| `src/interaction.rs` | Rover pickup, goal preview, release, cancellation, and navigation admission |
| `src/local_map.rs` | Local occupancy grid, routes, and grid coordinate conversion |
| `src/map_overlay.rs` | Geographic occupancy, routes, and destination overlays |
| `src/model_drag.rs` | Pure conversion from a drag destination to the preview model position |
| `src/occupancy.rs` | Lossless occupancy spans that reduce rendering work |
| `src/launch.rs` | Real/simulation launch selection |

Clicking a rover selects it and takes the window's **local** control claim. It does not arm the rover or acquire an exclusive remote lease. Control-drag moves a model preview while the ground marker retains the reported position. Release submits a goal through the existing admission checks; Escape cancels.

The `terra-map` crate owns the camera and tile renderer. A map drag only continues after a map press; pointer movement cannot acquire a gesture begun on a rover. Loaded tile triangles provide visual surface heights for model placement and map projection. The lookup includes roofs and canopies, so it is not a traversability map or navigation feedback. Without loaded surface data, placement falls back to the camera reference height.

Run library checks with `cargo test -p argos-gpui --lib` and build the UI with `cargo build -p argos-gpui`. Map tests live in `terra-map`; explicitly enabled GPU tests cover rendered surface heights and depth rendering.
