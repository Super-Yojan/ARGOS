# ARGOS

ARGOS (Adaptive Robotic Group Operator System) is the fleet operator system for Terra vehicles and the Zorvane simulator. The dashboard is a native GPUI application. `crates/terra-map` draws the Map View (wgpu Google Photorealistic 3D Tiles). `crates/argos-gpui` is the dashboard binary and mounts that view. A Rust Axum service owns authenticated operator sessions, command validation, audit history, and the Zenoh fleet connection. Terra owns sensing, onboard autonomy, motor control, and watchdogs.

As of October 19, 2026, new dashboard work targets `crates/argos-gpui` and `crates/terra-map`. The React app and the SwiftUI apps have been removed. The Unity experiment remains under `spikes/` as historical prototype work. TerraPhone's native vehicle runtime is a separate application and remains in use. This binary does not yet draw a fleet list or send Zenoh commands; the map is the first surface.

## Start the dashboard

Install stable Rust (edition 2024) and a C toolchain. Linux also needs the GPUI and Vulkan development packages (`libwayland`, `libxkbcommon`, `libvulkan`, and X11/XCB headers). No API key is stored in this repository.

`GOOGLE_MAPS_API_KEY` in the process environment loads Google tiles. The Google Cloud project needs billing and the Map Tiles API. Without a key, or for an offline check, use the synthetic fixture:

```sh
MAPS_DEMO=1 cargo run -p argos-gpui
```

With a key:

```sh
export GOOGLE_MAPS_API_KEY='your-key'
cargo run -p argos-gpui
```

The window title is ARGOS. Keyboard and pointer handling are the map's: drag to orbit in 3D, Shift-drag to pan, scroll to zoom, and 2 / 3 to switch modes. Map configuration, model conventions, and the standalone `learn-gpui` explorer are in [crates/terra-map/src/MAPS.md](crates/terra-map/src/MAPS.md).

## Service and fleet transport

`crates/argos-server` and `crates/argos-zenoh` stay. The server still authenticates operators and publishes validated commands; it no longer ships a browser bundle. Static files are served from `ARGOS_WEB_DIR` when that directory exists (default `web/dist`).

Local simulated service, loopback only:

```sh
cargo run -p argos-server -- --demo --dev
```

Docker builds that service without a frontend. From the repository root, with Docker running:

```sh
docker compose up -d --build
```

`http://127.0.0.1:8080/healthz` checks the process. There is no browser dashboard at that origin. Stop with `docker compose down`. Sessions and audit history persist in a Docker volume; `docker compose down -v` deletes that local data.

To connect the service to a real Zenoh fleet, use [compose.fleet.yaml](compose.fleet.yaml) and [authenticated hosting notes](docs/browser-dashboard.md#connect-a-real-fleet-from-local-docker). The GPUI app does not join that session yet.

- [Getting started](docs/getting-started.md)
- [Architecture and control boundary](docs/DESIGN.md)
- [Current status and remaining work](docs/roadmap.md)
- [Published documentation](https://super-yojan.dev/ARGOS/)

## Verify changes

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p terra-map
```

GPU tests in `terra-map` stay ignored. Run them only where a native GPU is available:

```sh
cargo test -p terra-map gpu_ -- --ignored
```

Live transport tests need permission to bind loopback sockets. `ARGOS_CHECK_TYPES=1` checks the committed TypeScript contract snapshot at `crates/argos-server/tests/generated.ts` without rewriting it. The Unity spike is unchanged and is not part of the dashboard.

## Workspace

| Crate | Role |
| --- | --- |
| `crates/argos-gpui` | Dashboard binary. Opens a window and mounts `MapView`. |
| `crates/terra-map` | Map library (`terra_map`) and the `learn-gpui` explorer. |
| `crates/argos-server` | HTTP sessions, command validation, audit, fleet worker. |
| `crates/argos-zenoh` | Zenoh client, router binary, session log. |
| `crates/argos-core` | Shared contracts and operator state. |
| `crates/argos-ffi` | UniFFI facade. The Swift dashboard that called it is gone. |
| `spikes/unity-dashboard` | Historical prototype. |

### Change-driven visualization telemetry

Updated Terra and Zorvane publishers send full occupancy/cloud payloads when their content changes, with periodic recovery baselines for late subscribers. New live observations of unchanged content can instead send `map/occupancy/heartbeat` or `pointcloud/heartbeat`. These small messages identify the last full rover/run or frame/sequence; the Rust client renews only that matching cached visualization. Heartbeats never create missing data, authorize driving, or renew hardware/authority state. A stalled sensor emits neither fresh content nor heartbeats.

ARGOS retains compatibility with full-payload publishers. Deploy updated clients alongside change-driven publishers: older receivers that do not understand these heartbeats will mark unchanged maps stale between recovery baselines. Zorvane currently publishes occupancy and depth/RGB, not a separate point-cloud stream.
