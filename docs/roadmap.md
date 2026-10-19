# Current status and roadmap

Updated October 19, 2026. This page describes repository implementation; builds and simulator tests do not establish physical-vehicle performance or completed research trials.

## Primary direction

The operator surface is the GPUI app in `crates/argos-gpui`. It opens a window and mounts `terra_map::MapView`. The React dashboard and the SwiftUI apps have been removed. `argos-server` and Zenoh remain the session and transport path. The Unity spike is preserved for reference and is not the dashboard.

| Area | Current state |
| --- | --- |
| GPUI dashboard | One window. Map View, live rover markers, local occupancy, drag-to-goal, autonomy levels, and held teleop |
| Map library | `crates/terra-map` (`terra_map`): Google 3D Tiles, local glTF/GLB instances, offline `MAPS_DEMO=1` fixture |
| Operator access | Service still has GitHub OAuth allowlists, viewer/operator roles, per-tab control claim, CSRF checks |
| Vehicle commands | Service still validates waypoint send/cancel, arm/disarm, autonomy selection, held driving, and latched stop. The GPUI window sends goal, cancel, autonomy, teleop, and safety stop on the Zenoh keys Terra already consumes. It does not arm hardware |
| Navigation tuning | Service retains dimensions, clearance, planner speed/turn/look-ahead with revision acknowledgement |
| Mapping | Rust client still validates occupancy run/frame and content heartbeats. The sidebar draws the local grid. Occupied means 65 or higher |
| Hosting | Docker/Caddy and GCP packaging build `argos-server` without a browser bundle |
| Removed UI | `apps/web` and `apps/apple` are gone. `argos-ffi` remains in the workspace |
| Unity | Historical prototype under `spikes/`; not the dashboard platform |
| Staff model / VLA | Research and experiments; not a deployed natural-language fleet planner |

## Next work and validation

- The GPUI window recenters on live rover positions. Local goals send when the operator releases a drag. Outdoor GPS goals are drafted by a map click and sent by a separate confirm, and only after an acknowledged `outdoor_gps` capability.
- Keep the offline `MAPS_DEMO=1` fixture working when the router is down.
- Validate real vehicle braking, control latency, phone lifecycle, and multi-client operator discipline against the service. Do not substitute demo results or the offline map fixture for physical evidence.
- Keep study protocols and evidence in Masters-Thesis aligned with the interface that was actually tested. The React and Swift trees are no longer in this repository.

## Branch and documentation policy

`main` is the integration baseline. Merge reviewed feature PRs after relevant checks, then branch new work from updated main. Retain branches with unique commits until their work is explicitly reconciled; do not delete active worktrees or uncommitted experiments to make the branch list shorter.

Older native, browser, and Unity plans, screenshots, and measurements remain historical records with status notices. The [getting-started guide](getting-started.md) and [architecture](DESIGN.md) describe the active window and service. The [removed browser dashboard](browser-dashboard.md) records the React controls. The [project board](https://github.com/users/Super-Yojan/projects/6) tracks follow-up work; old dated board snapshots are not current status.
