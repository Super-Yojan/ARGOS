# Current status and roadmap

Updated October 17, 2026. This page describes repository implementation; builds and simulator tests do not establish physical-vehicle performance or completed research trials.

## Primary direction

ARGOS is moving to a web-based operator dashboard. React/TypeScript and the Rust service are the primary path. SwiftUI is retained as a legacy fallback. The Unity spike is preserved for reference, but the proposed Unity C3 migration is superseded.

| Area | Current state |
| --- | --- |
| Browser dashboard | Shared fleet scene, local/geographic maps, rover selection, unknown-frame handling |
| Operator access | GitHub OAuth allowlists, viewer/operator roles, per-tab control claim, CSRF checks |
| Vehicle commands | Reviewed waypoint send/cancel, explicit arm/disarm, supported autonomy selection, held manual driving and latched stop |
| Navigation tuning | Dimensions, clearance, planner speed/turn/look-ahead with revision acknowledgement |
| Mapping | Selected-rover occupancy with run/frame validation and freshness; content-heartbeat support in Rust client |
| Hosting | Docker/Caddy and GCP service packaging; deployment health requires live verification |
| Native fallback | Existing manual driving, stop, proposal and mission flows retained |
| Unity | Historical prototype under `spikes/`; not the next dashboard platform |
| Staff model / VLA | Research and experiments; not a deployed natural-language fleet planner |

## Next work and validation

- Validate the browser controls on physical hardware after the automated demo/transport checks. Safety reset remains in the native fallback; it has not yet moved to the browser.
- Add browser target-search mission setup and proposal/review workflows where supported by Terra.
- Validate real vehicle braking, control latency, phone lifecycle, and multi-client operator discipline. Do not substitute demo results for physical evidence.
- Evaluate shared 3D world work independently: the local `codex/shared-3d-world` branch contains additional work not included in the current open-PR consolidation. Its existence does not make it part of published main.
- Keep study protocols and evidence in Masters-Thesis aligned with the actual tested interface.

## Branch and documentation policy

`main` is the integration baseline. Merge reviewed feature PRs after relevant checks, then branch new work from updated main. Retain branches with unique commits until their work is explicitly reconciled; do not delete active worktrees or uncommitted experiments to make the branch list shorter.

Older native/Unity plans, screenshots, and measurements remain historical records with status notices. The [browser guide](browser-dashboard.md), [architecture](DESIGN.md), and [getting-started guide](getting-started.md) describe the active interface. The [project board](https://github.com/users/Super-Yojan/projects/6) tracks follow-up work; old dated board snapshots are not current status.
