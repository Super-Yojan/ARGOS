# Current status and roadmap

Updated October 21, 2026. This page describes repository implementation; builds and simulator tests do not establish physical-vehicle performance or completed research trials.

## Primary direction

The operator surface is the GPUI app in `crates/argos-gpui`. It opens a window and mounts `terra_map::MapView`. The React dashboard and the SwiftUI apps have been removed. The desktop app connects directly to Zenoh; the HTTP server is removed. The Unity spike is preserved for reference and is not the dashboard.

| Area | Current state |
| --- | --- |
| GPUI dashboard | One window. Map View, live rover markers, local occupancy, drag-to-goal, hold-to-arm, autonomy levels, and held teleop |
| Map library | `crates/terra-map` (`terra_map`): Google 3D Tiles, local glTF/GLB instances, offline `MAPS_DEMO=1` fixture |
| Operator access | Fleet membership credentials; local desktop control claim; no HTTP roles or OAuth |
| Vehicle commands | The GPUI window sends goal, cancel, autonomy, teleop, safety stop, safety reset, and the existing hardware arm/disarm payload. A one-second hold is required before arm. Reset does not arm |
| Navigation tuning | Vehicle runtime owns navigation configuration and command admission |
| Mapping | Rust client still validates occupancy run/frame and content heartbeats. The main map draws the local grid under each rover. Occupied means 65 or higher |
| Hosting | GCP authenticated Zenoh router with Caddy certificate renewal; local router Docker image |
| Removed UI | `apps/web` and `apps/apple` are gone. `argos-ffi` remains in the workspace |
| Unity | Historical prototype under `spikes/`; not the dashboard platform |
| Staff model / VLA | Research and experiments; not a deployed natural-language fleet planner |

## Next work and validation

- The GPUI window recenters on live rover positions. Local and outdoor GPS goals both send when the operator releases a drag. Outdoor release is the explicit action and replaces the spec's confirm step. It still requires an acknowledged `outdoor_gps` capability, and the vehicle runtime checks command admission.
- Keep the offline `MAPS_DEMO=1` fixture working when the router is down.
- Validate real vehicle braking, control latency, phone lifecycle, and multi-client operator discipline with the direct Zenoh path. Do not substitute demo results or the offline map fixture for physical evidence.
- Keep study protocols and evidence in Masters-Thesis aligned with the interface that was actually tested. The React and Swift trees are no longer in this repository.

## Branch and documentation policy

`main` is the integration baseline. Merge reviewed feature PRs after relevant checks, then branch new work from updated main. Retain branches with unique commits until their work is explicitly reconciled; do not delete active worktrees or uncommitted experiments to make the branch list shorter.

Older native, browser, and Unity plans, screenshots, and measurements remain historical records with status notices. The [getting-started guide](getting-started.md) and [architecture](DESIGN.md) describe the active window and router. The [removed browser dashboard](browser-dashboard.md) records the React controls. The [project board](https://github.com/users/Super-Yojan/projects/6) tracks follow-up work; old dated board snapshots are not current status.
