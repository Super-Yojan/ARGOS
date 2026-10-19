# ARGOS

ARGOS (Adaptive Robotic Group Operator System) is the fleet operator system for Terra vehicles and the Zorvane simulator. The dashboard is a native GPUI application. `crates/terra-map` draws the Map View (wgpu Google Photorealistic 3D Tiles). `crates/argos-gpui` is the dashboard binary and mounts that view. A Rust Axum service owns authenticated operator sessions, command validation, audit history, and the Zenoh fleet connection. Terra owns sensing, onboard autonomy, motor control, and watchdogs.

As of October 19, 2026, new dashboard work targets `crates/argos-gpui` and `crates/terra-map`. The React app and the SwiftUI apps have been removed. The Unity experiment remains under `spikes/` as historical prototype work. TerraPhone's native vehicle runtime is a separate application and remains in use. The window subscribes to Zenoh and draws each rover's position and heading. It does not publish goals, teleop, or other commands.

## Start the dashboard

Install stable Rust (edition 2024) and a C toolchain. Linux also needs the GPUI and Vulkan development packages (`libwayland`, `libxkbcommon`, `libxkbcommon-x11`, `libvulkan`, and X11/XCB headers). No API key is stored in this repository.

`GOOGLE_MAPS_API_KEY` in the process environment loads Google tiles. The Google Cloud project needs billing and the Map Tiles API. Without a key, or for an offline check, use the synthetic fixture:

```sh
MAPS_DEMO=1 cargo run -p argos-gpui
```

With a key:

```sh
export GOOGLE_MAPS_API_KEY='your-key'
cargo run -p argos-gpui
```

The window title is ARGOS. A sidebar lists the Zenoh link and each rover. Keyboard and pointer handling are the map's: drag to orbit in 3D, Shift-drag to pan, scroll to zoom, and 2 / 3 to switch modes when the map controls are shown. Map configuration, model conventions, and the standalone `learn-gpui` explorer are in [crates/terra-map/src/MAPS.md](crates/terra-map/src/MAPS.md).

The camera opens at a fallback location. It is not locked to a campus. Once one or more rovers have a position, the view recenters on their centroid, and it recenters again only when that set of rovers changes. Pose updates after that leave the camera where the operator put it. With no rover seen yet, the fallback is `ARGOS_MAP_START_LAT` / `ARGOS_MAP_START_LON`. Unset, those default to GMU Fairfax (38.8297, -77.3075).

| Variable | Default | Role |
| --- | --- | --- |
| `ARGOS_MAP_START_LAT`, `ARGOS_MAP_START_LON` | 38.8297, -77.3075 | Camera until a rover position arrives |
| `ARGOS_MAP_ORIGIN_LAT`, `ARGOS_MAP_ORIGIN_LON` | the start pair | Local-metre origin when Terra has not published a geographic alignment |
| `ARGOS_ZENOH_ENDPOINT` | `tcp/127.0.0.1:7447` | Router to subscribe to. `tcp/` or `tls/` |
| `ARGOS_ZENOH_PREFIX` | `terra/rover` | Key prefix. TerraPhone and the hosted fleet use `terra/phone` |
| `ARGOS_ZENOH_USER`, `ARGOS_ZENOH_PASSWORD` | unset | Set both, or neither. `tls/` uses the system trust store |
| `MAPS_DEMO` | unset | `1` draws the offline fixture and does not call Google |
| `GOOGLE_MAPS_API_KEY` | unset | Photorealistic 3D Tiles. Not stored in this repo |

A bad number, a half-set pair, or an endpoint that is not `tcp/` or `tls/` prints the reason and exits before the window opens. A router that is down does not: the window still opens on the start location and the sidebar shows the connection error. `MAPS_DEMO=1` keeps working in that case.

Rover `x`/`y` are local metres. When the rover's localization report is version 1, mode `geographic`, and includes `originLatitude` / `originLongitude`, the marker uses that frame, including `anchorX`, `anchorY`, and `rotation`. Otherwise the marker uses `ARGOS_MAP_ORIGIN_*`. GPS fields on a local-mode report stay observations; they do not move the marker. Heading on the map is clockwise degrees. Terra yaw is radians from +x toward +y.

Markers are the `robot.glb` model at 12× scale and 80 m ellipsoid height so they stay visible at neighborhood zoom. That height is display exaggeration, not ground clamping. Online rovers with a pose newer than 2.5 s draw in the model color. Stale and offline rovers are tinted. The sidebar summarizes occupancy (`width x height`, occupied cells, age). The map does not paint the grid.

`argos-gpui` only subscribes. Fleet membership still has to arrive on `<prefix>/fleet/state` before a pose is kept.

Zorvane's router listens on `tcp/127.0.0.1:7447` with prefix `terra/rover`, which matches the defaults. A TerraPhone or the ARGOS server's loopback router is often `tcp/127.0.0.1:7448`. The hosted fleet router is `tls/argos.34.135.204.210.sslip.io:7448` with prefix `terra/phone` and user `terra`. Take the password from the operator secret store (`/etc/argos/fleet.password` on the deployed host). Do not put it in the repo.

```sh
ARGOS_ZENOH_ENDPOINT=tls/argos.34.135.204.210.sslip.io:7448 \
ARGOS_ZENOH_PREFIX=terra/phone \
ARGOS_ZENOH_USER=terra \
ARGOS_ZENOH_PASSWORD='from-the-secret-store' \
cargo run -p argos-gpui
```

`pi@terra1.local` runs the `terra-rover` BLE and motor service. That process does not publish Zenoh poses. Poses come from TerraPhone or from a Zenoh router on the LAN. If that host is running the router:

```sh
ARGOS_ZENOH_ENDPOINT=tcp/terra1.local:7447 \
ARGOS_ZENOH_PREFIX=terra/rover \
cargo run -p argos-gpui
```

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

To connect the service to a real Zenoh fleet, use [compose.fleet.yaml](compose.fleet.yaml) and [authenticated hosting notes](docs/browser-dashboard.md#connect-a-real-fleet-from-local-docker). The GPUI app joins the same bus directly. It does not sign into the HTTP service.

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
| `crates/argos-gpui` | Dashboard. Mounts `MapView` and subscribes, read-only, to Zenoh. |
| `crates/terra-map` | Map library (`terra_map`) and the `learn-gpui` explorer. |
| `crates/argos-server` | HTTP sessions, command validation, audit, fleet worker. |
| `crates/argos-zenoh` | Zenoh client, router binary, session log. |
| `crates/argos-core` | Shared contracts and operator state. |
| `crates/argos-ffi` | UniFFI facade. The Swift dashboard that called it is gone. |
| `spikes/unity-dashboard` | Historical prototype. |

### Change-driven visualization telemetry

Updated Terra and Zorvane publishers send full occupancy/cloud payloads when their content changes, with periodic recovery baselines for late subscribers. New live observations of unchanged content can instead send `map/occupancy/heartbeat` or `pointcloud/heartbeat`. These small messages identify the last full rover/run or frame/sequence; the Rust client renews only that matching cached visualization. Heartbeats never create missing data, authorize driving, or renew hardware/authority state. A stalled sensor emits neither fresh content nor heartbeats.

ARGOS retains compatibility with full-payload publishers. Deploy updated clients alongside change-driven publishers: older receivers that do not understand these heartbeats will mark unchanged maps stale between recovery baselines. Zorvane currently publishes occupancy and depth/RGB, not a separate point-cloud stream.
