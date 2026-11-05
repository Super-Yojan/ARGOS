# GPUI dashboard code

The library receives fleet telemetry; the executable renders it and handles operator input.

The code is in two halves. `src/main.rs` plus `src/ui/` is the window. Every other file in `src/` is the library: fleet data, settings, and sending commands.

### The window

| File | Responsibility |
| --- | --- |
| `src/main.rs` | The `Dashboard` state struct and `main` (theme, window) |
| `src/ui/state.rs` | Startup, the 100 ms sync from `FleetFeed`, 3D rover markers, fleet switching |
| `src/ui/actions.rs` | What buttons and keys do: select, stop, drive, study condition, explore, task |
| `src/ui/render.rs` | Window layout and its mouse and key handlers |
| `src/ui/console_bar.rs` | Top bar: map mode, grid, status, control claim, link, Stop all |
| `src/ui/cards.rs` | Rover callout cards and off-screen tabs |
| `src/ui/popups.rs` | Dump approval and click-to-goal confirm popups |
| `src/ui/mission.rs` | Mission tab: planned steps per rover, ticking each running mission, map step markers |
| `src/ui/inspector.rs` | The selected rover's panel and its Overview tab |
| `src/ui/levels.rs` | Autonomy level buttons and the study condition |
| `src/ui/task.rs` | L4 Explore and L5 Task sections |
| `src/ui/sections.rs` | Goal, bucket dock, and 3D capture sections |
| `src/ui/drive.rs` | Teleop section, turning limits, W/A/S/D pad |
| `src/ui/video.rs` | FPV video tab |
| `src/ui/widgets.rs` | Shared section titles, hints, cautions, rows |
| `src/ui/agent.rs` | Agent bridge, recommendation cards, raw-drive grant |
| `src/ui/interaction.rs` | Rover pickup, goal preview, release, cancellation, and navigation admission |
| `src/ui/map_overlay.rs` | Geographic occupancy, routes, and destination overlays |
| `src/ui/local_map.rs` | Occupancy cell colors and dotted strokes |
| `src/ui/palette.rs` | Console colors |

### The library

| File | Responsibility |
| --- | --- |
| `src/fleet/config.rs` | Defaults, `.env` loading, `DashboardConfig`, log and capture folders |
| `src/fleet/marks.rs` | Zenoh snapshot → one `RoverMark` per rover, presence rows, camera framing |
| `src/fleet/feed.rs` | `FleetFeed`, the window's handle: start, latest picture, teleop limits |
| `src/fleet/commands.rs` | Every command sent to a rover (levels, study, task, dock, goals, drive, stops, capture) |
| `src/fleet/poll.rs` | The `argos-fleet` thread: connect, reconnect, refresh every 100 ms |
| `src/console.rs` | C3 console presentation: each rover's field callout, attention, inspector trace and sections, unlocated fleet members, and field notices |
| `src/model_drag.rs` | Pure conversion from a drag destination to the preview model position |
| `src/occupancy.rs` | Lossless occupancy spans that reduce rendering work |
| `src/launch.rs` | Real/simulation launch selection |
| `src/selection.rs` | Keep a rover selected through a short reporting dropout |
| `src/fpv/` | Receive-only rover camera |
| `src/agent_bridge.rs`, `src/agent_mcp.rs`, `src/agent_runner.rs` | Agent socket, built-in MCP server, coding-agent CLIs |

Clicking a rover selects it and takes the window's **local** control claim. It does not arm the rover or acquire an exclusive remote lease. With a rover selected, a plain click on the main map proposes a destination there and opens a small popup at the click. **Send goal** submits it through the existing admission checks; **Cancel** or Escape sends nothing. The rover model stays at its reported position. Releasing control deselects the rover and closes its panel. The local map tab is display-only.

The `terra-map` crate owns the camera and tile renderer. A map drag only continues after a map press; pointer movement cannot acquire a gesture begun on a rover. Loaded tile triangles provide visual surface heights for model placement and map projection. The lookup includes roofs and canopies, so it is not a traversability map or navigation feedback. Without loaded surface data, placement falls back to the camera reference height.

Run library checks with `cargo test -p argos-gpui --lib` and build the UI with `cargo build -p argos-gpui`. Map tests live in `terra-map`; explicitly enabled GPU tests cover rendered surface heights and depth rendering.
