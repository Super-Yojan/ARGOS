# Browser ARGOS implementation plan

> **Status — October 17, 2026:** This is a historical design, implementation record, or legacy workflow. The React dashboard and SwiftUI apps were removed on October 19, 2026. The operator surface is the GPUI app in `crates/argos-gpui`, which mounts the `terra-map` Map View. The HTTP service was removed on October 21, 2026; the desktop app uses Zenoh directly. Read the [removed browser dashboard](../../browser-dashboard.md) for the retirement notice. Unity C3 proposals are superseded; recorded evidence remains specific to its original platform and date.

The approved conversation plan is the specification. Goal: one hosted team, GitHub login, monitoring and local/geographic waypoint controls; development on all three desktop platforms without Apple tools.

Architecture: React/TypeScript/Vite frontend, Rust/Axum service with a dedicated synchronous fleet worker, existing Zenoh transport, SQLite, GitHub OAuth, Caddy and private Tailscale connectivity.

## Tasks
- [x] Service contracts and deterministic command state machine; test readiness, stale inputs, request deduplication, mode acknowledgment, claim expiry, cancellation and reconnect.
- [x] Axum API, SSE, SQLite audit/sessions, GitHub login and secure production configuration; test authorization and HTTP mutation boundaries.
- [x] React dashboard, SVG local map and MapLibre geographic map; test offline maps, typed coordinates, confirmation and status.
- [x] Cross-platform development commands, CI, container deployment and operating documentation.
- [x] Independent review; fixes verified with regression tests. Final workspace verification recorded below.

## Decisions and progress
- Work on feature branch codex/browser-dashboard in the supplied checkout; branch creation required sandbox approval.
- Existing baseline passed core tests; local socket tests require elevated test execution.
- Use SSE (not WebSockets) because this release only needs server-pushed telemetry and HTTP commands.
- SQLite persists request IDs before publication; unfinished operations become interrupted on server startup and are never resumed.
- Keep native applications untouched. The browser's ownership claim does not fence native clients.

## Review and validation
- Review fixes: release/expired claims immediately retire unpublished intent; local frame IDs survive without geographic alignment; requests/drafts bind rover run and frame; cancellation requires a new idle observation; externally superseded accepted operations terminate; pending geographic commands recheck alignment quality.
- Rust service tests cover HTTP roles/CSRF/session expiry, persistent restart recovery, command state policy, and a synthetic rover over real local Zenoh transport.
- Frontend: eight unit tests and five Chromium integration tests, including geographic rendering, explicit confirmation, arrival/cancel, offline clearing, competing tabs and narrow screens. Dashboard screenshot inspected.
- npm audit: zero vulnerabilities. TypeScript production build passes. Compose configuration validates.
- Deployment limitations: no running Docker daemon, domain/OAuth credentials, Windows/Linux execution host, physical hardware, or Zorvane instance used in this session. Those checks remain part of rollout; cross-platform CI is supplied.
- Final Rust workspace verification: 59 tests passed; Clippy with warnings denied, formatting, generated type drift, and whitespace checks passed. Geographic alignment late-arrival regression included in frontend tests.
