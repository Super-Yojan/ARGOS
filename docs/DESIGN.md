# ARGOS architecture and control boundary

ARGOS is a web fleet operator station. React/TypeScript renders observations and operator controls; the Rust Axum service validates authenticated requests and owns one fleet worker. Terra owns sensing, onboard planning, final motor selection, and watchdogs.

```mermaid
flowchart LR
  web[React browser] -->|HTTPS / SSE| service[argos-server]
  service --> store[(SQLite sessions and audit)]
  service --> zenoh[argos-zenoh]
  zenoh --> core[argos-core]
  zenoh --> bus[Zenoh fleet]
  legacy[Legacy Swift app] --> ffi[argos-ffi / UniFFI] --> zenoh
```

| Component | Responsibility |
| --- | --- |
| `apps/web` | Fleet scene, selection, reviewed commands, browser interaction |
| `crates/argos-server` | OAuth/session roles, CSRF/origin checks, claims, command engine, SSE, persistence |
| `crates/argos-core` | Contracts, projection, freshness, acknowledgement and operator state |
| `crates/argos-zenoh` | Fleet transport, validated telemetry, command publication, session logs |
| `crates/argos-ffi` and `apps/apple` | Retained native fallback |
| `spikes/unity-dashboard` | Historical Unity prototype; not the selected product UI |
| `packaging/web`, `packaging/gcp` | Hosted service/router packaging |

## Observation and command semantics

The service broadcasts fleet snapshots at 5 Hz. Large occupancy arrays are fetched separately for the selected rover. IDs and sequence values crossing the browser boundary use strings to preserve 64-bit precision. Unknown pose and incompatible frames remain visibly unknown; geographic conversion requires reported alignment.

Fleet supervision and waypoint submission use a 90-second freshness horizon; goal acknowledgement waits up to 150 seconds. Manual/hardware controls enforce stricter freshness. Occupancy display has its own 2.5-second stale threshold. These are receipt ages, not a claim of synchronized end-to-end sensor time.

Waypoint requests are reviewed against run/frame identity and checked again before publication. A matching vehicle token confirms acceptance; publication alone does not. Cancel waits for a newly observed idle report. No reconnect or restart replays commands, and a published vehicle goal can outlive the browser session.

Hardware arm is explicit, short-lived, and bound to the observed run/authority revision. Disarm remains available with stale observations. Geometry/avoidance configuration requires disarmed ready hardware and is acknowledged separately from a request being sent. Mode selection never arms the vehicle.

Held browser driving uses fresh manual authority, ordered pulses, queue expiry, and a server-side deadman. Emergency stop attempts the vehicle safety latch, cancellation, and disarm; publication is not physical-stop acknowledgement. Safety reset remains in the legacy native interface.

## Authentication and ownership

GitHub OAuth identifies invited numeric account IDs as viewer or operator. Hosted cookies are secure and HTTP-only; mutations require same-origin and CSRF validation. A browser tab renews a 30-second command claim every ten seconds. SQLite persists sessions and command/audit records; claims remain in memory, so production uses one instance.

The browser claim does not fence native clients on Zenoh. Avoid simultaneous native/browser operators. The vehicle remains the final authority on whether a request can execute.

## Migration status

The web interface is the primary development target. Native mission proposal approval, target-search setup, and native evidence/export flows are not automatically browser features. See the [browser guide](browser-dashboard.md) for implemented controls and the [roadmap](roadmap.md) for remaining migration work. Historical plans describe their original slice and are not the current architecture.
