# ARGOS

ARGOS (Adaptive Robotic Group Operator System) is the **web-based fleet operator dashboard** for Terra vehicles and the Zorvane simulator. React/TypeScript supplies the interface; a Rust Axum service owns authenticated operator sessions, command validation, audit history, and the Zenoh fleet connection. Terra owns sensing, onboard autonomy, motor control, and watchdogs.

As of October 17, 2026, new dashboard work targets `apps/web`. The SwiftUI apps are retained as a legacy fallback; the Unity experiment remains under `spikes/` as historical prototype work. Neither is the target for new dashboard features. TerraPhone's native vehicle runtime is a separate application and remains in use.

## Start the web dashboard

With Docker Desktop running (or Docker Engine with Compose on Linux), run from
the repository root:

```sh
docker compose up -d --build
```

Open **http://127.0.0.1:8080** and choose **Open local demo**. This builds the
dashboard and Rust service and starts a simulated fleet without installing Rust
or Node.js on your host. The first build downloads and compiles dependencies.
Only the local computer can access this demo; no GitHub credentials are needed.

View logs with `docker compose logs -f`, and stop with `docker compose down`.
Sessions and audit history persist in a Docker volume. To reset all local demo
data, use `docker compose down -v`.
See [Docker deployment details](docs/browser-dashboard.md#local-docker-deployment)
for how the local setup works and the separate authenticated hosting option.

To connect actual Terra vehicles, use the separate
[authenticated local fleet setup](docs/browser-dashboard.md#connect-a-real-fleet-from-local-docker).
It supports the hosted Terra router, a LAN router, or a router on this computer.

### Develop without Docker

Install stable Rust and Node.js 22.12 or newer. Windows, Linux, and macOS development is supported.

```sh
npm ci --prefix apps/web
npm run dev
```

Open `http://127.0.0.1:5173` and choose **Open local demo**. The built-in simulated fleet needs no rover, account, or Xcode. Select a rover on the map and take control to use the operator panel. Real fleets use the authenticated service deployment, not development identity.

The browser provides a shared fleet field, local and geographic maps, occupancy overlays, reviewed waypoint send/cancel, explicit hardware arm/disarm, held keyboard/touch manual driving, latched emergency stop, autonomy selection, and vehicle geometry/avoidance configuration. The geographic view displays raw GPS without local alignment, with a default street basemap and an optional HTTPS tile style override. Unknown positions remain visibly unknown.

- [Browser operation and hosting](docs/browser-dashboard.md)
- [Getting started](docs/getting-started.md)
- [Architecture and control boundary](docs/DESIGN.md)
- [Current status and remaining work](docs/roadmap.md)
- [Published documentation](https://super-yojan.dev/ARGOS/)

## Verify changes

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
npm test --prefix apps/web
npm run build --prefix apps/web
npm run test:e2e --prefix apps/web
```

Install Playwright Chromium before the first browser test run. Live transport tests need permission to bind loopback sockets. Apple and Unity UI validation require their own platform tools; Rust tests do not certify those interfaces or physical driving.

## Legacy Apple fallback

The Apple dashboard uses a shared Rust Zenoh client through UniFFI and native
SwiftUI/MapKit views. ARGOS supplies four autonomy levels, supervised proposal approval, per-rover/fleet
takeover and emergency stop, held keyboard/touch teleop, and session-log export.
The [mission-control guide](docs/autonomy/README.md) covers the linked runtime and evidence.
ARGOS discovers Terra rovers, displays live positions and
goal progress, and sends latched high-level waypoint/cancel commands.

Requirements: macOS with Xcode, Rust (edition 2024), the `xcodeproj` Ruby gem,
and Apple targets installed with `rustup`. The initial package supports Apple
Silicon Macs, physical iOS devices, and Apple Silicon iOS simulators. Deployment
floors are macOS 14 and iOS 17.

```sh
rustup target add aarch64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim
gem install xcodeproj
./scripts/build-apple.sh
# The Xcode project is committed. Regenerate it only when adding/removing sources:
# ruby scripts/create-apple-project.rb
open apps/apple/ARGOS.xcodeproj
```

The build script packages optimized Rust libraries by default, independently of
Xcode's Debug/Release selection. Use `ARGOS_RUST_PROFILE=debug ./scripts/build-apple.sh`
when you specifically need an unoptimized Rust build.

Select `ARGOSMac` for a native Mac app or `ARGOSiOS` for iPhone/iPad. Device builds
need your Apple signing team. The macOS target has outgoing-network sandbox
permission; iOS describes its local-network access request. Allow local-network
access when prompted.

Start Zorvane, the world simulator, in another terminal. A geographic demo
using its bundled GMU anchor is:

```sh
cd /path/to/Zorvane
TERRA_ROVER_COUNT=1 TERRA_TILES=1 TERRA_TILES_FETCH=0 cargo run -p zorvane
```

In ARGOS, open **Connection**, set endpoint `tcp/127.0.0.1:7447`, prefix
`terra/rover`, and enable **Geographic map**. Keep latitude `38.8297`, longitude
`-77.3075` to match that simulator. Select a rover, tap/click a target or enter
latitude `38.82981`, longitude `-77.3075`, explicitly select **Waypoint**, then choose **Send waypoint**. The app
shows the correlation token, acceptance, distance remaining, and arrival.
For the default flat practice world, disable Geographic map and enter local
x/y coordinates in metres; +x is north/forward, +y is west/left. The local plot
supports zooming without an internet basemap.

A physical phone needs your simulator computer's LAN address, not phone
localhost. Start Zorvane with `TERRA_ZENOH_LISTEN=tcp/0.0.0.0:7447` and enter
`tcp/COMPUTER_LAN_IP:7447` in the app. The app's configured anchor must match
Terra; the bus does not advertise the anchor or whether tile loading succeeded.

**Send means published, not accepted.** A matching Terra status confirms the
UUID token. After 150 seconds without confirmation the app shows unconfirmed;
it does not resend. Cancel sends one cancellation and waits for idle. This
releases the latched goal for debug teleop; it does not send a teleop command.
Closing, disconnecting, or backgrounding the app does not cancel Terra's goal.
Observation resumes with fresh receive ages; commands are never replayed by ARGOS.

Pose comes from exposure-aligned `body` metadata in depth packets. The app
receives depth packets but discards their pixels, so depth-network bandwidth is
still present. Pose, goal status, and fleet membership have independent ages;
Fleet supervision uses a 90-second horizon; manual control and hardware actions use stricter freshness checks. Missing pose stays unknown. Terra goal-status x/y denote
the destination. MapKit basemap imagery needs network access; local controls do
not. L2 direct waypoint does not avoid obstacles; L3 uses the onboard obstacle-aware planner.

### Native checks

```sh
cargo test --workspace --locked
xcodebuild -project apps/apple/ARGOS.xcodeproj -scheme ARGOSMac \
  -destination 'platform=macOS,arch=arm64' -derivedDataPath apps/apple/build \
  test -only-testing:ARGOSMacTests CODE_SIGNING_ALLOWED=NO
xcodebuild -project apps/apple/ARGOS.xcodeproj -scheme ARGOSiOS \
  -destination 'platform=iOS Simulator,name=iPhone 18 Pro' \
  -derivedDataPath apps/apple/build-ios test -only-testing:ARGOSiOSTests \
  CODE_SIGNING_ALLOWED=NO
```

Choose an installed simulator name on your machine. With Terra already running
in the geographic demo, run `./scripts/check-swift.sh` for real arrival and
cancel through generated bindings. The UI-test targets also require that live
simulator; run `-only-testing:ARGOSMacUITests` or `-only-testing:ARGOSiOSUITests`
instead of the unit-test target. Their retained screenshot attachments provide
native UI evidence. Do not run both live command tests concurrently.

Codespaces can run Rust tests and Terra, but native Apple UI builds/runs require
Xcode on macOS. The browser dashboard has its own cross-platform development workflow above.

### Hardware arming from ARGOS

Select the Terra rover in the 3D scene and use **Arm** in its panel. **Disarm** remains available during pending arming or missing authority telemetry. Terra may stay on its home screen; it forwards explicit requests to its paired rover and publishes confirmed hardware status. Driving and waypoint execution require fresh confirmed armed status. Waypoint drafting remains available while disarmed.

Arm requests are bound to the current rover run and authority revision and expire after 500 ms. Both endpoints and the final phone execution check enforce freshness. No reconnect, mode selection, or drive takeover automatically arms hardware. Existing background stops, Bluetooth acknowledgement, emergency-stop and rover watchdog protections remain in effect.

### Spatial dashboard and router

The native Mac/iPad dashboard uses a dark spatial scene, on-demand robot inspection and camera orbit. Mac Connection settings can start a local Zenoh router with loopback, LAN (`0.0.0.0:7448`) or Tailscale sharing. iPad connects to a remote host or discovers Mac LAN routers. See [hybrid router setup](docs/deployment-router.md) and [shared scene contract](docs/design/spatial-scene-contract.md).

Public GCP deployment and Terra connection setup: [hosting guide](packaging/gcp/README.md).
### Change-driven visualization telemetry

Updated Terra and Zorvane publishers send full occupancy/cloud payloads when their
content changes, with periodic recovery baselines for late subscribers. New live
observations of unchanged content can instead send `map/occupancy/heartbeat` or
`pointcloud/heartbeat`. These small messages identify the last full rover/run or
frame/sequence; ARGOS renews only that matching cached visualization. Heartbeats
never create missing data, authorize driving or renew hardware/authority state.
A stalled sensor emits neither fresh content nor heartbeats.

ARGOS retains compatibility with full-payload publishers. Deploy updated ARGOS
alongside change-driven publishers: older receivers that do not understand these
heartbeats will mark unchanged maps stale between recovery baselines. Zorvane
currently publishes occupancy and depth/RGB, not a separate point-cloud stream.

Occupancy rendering uses cached textures rather than per-cell polygons on each
camera movement; the cache is bounded to 32 MiB including retained cell arrays.
Unknown-cell dots now scale with the grid. See [runtime update details](apps/apple/Shared/README.md#runtime-updates).
