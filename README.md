# ARGOS

ARGOS (Adaptive Robotic Group Operator System) is the fleet operator system for Terra vehicles and the Zorvane simulator. The dashboard is a native GPUI application. `crates/terra-map` draws the Map View (wgpu Google Photorealistic 3D Tiles). `crates/argos-gpui` is the dashboard binary and mounts that view. A Rust Axum service owns authenticated operator sessions, command validation, audit history, and the Zenoh fleet connection. Terra owns sensing, onboard autonomy, motor control, and watchdogs.

As of October 19, 2026, new dashboard work targets `crates/argos-gpui` and `crates/terra-map`. The React app and the SwiftUI apps have been removed. The Unity experiment remains under `spikes/` as historical prototype work. TerraPhone's native vehicle runtime is a separate application and remains in use. The window subscribes to Zenoh and draws each rover's position and heading. For a selected rover it can publish an autonomy level, a drag-to-goal waypoint, and held teleop. Terra still owns sensing, planning, and the motor limits.

## Start the dashboard

Install stable Rust (edition 2024) and a C toolchain. Linux also needs the GPUI and Vulkan development packages (`libwayland`, `libxkbcommon`, `libxkbcommon-x11`, `libvulkan`, and X11/XCB headers). No API key or fleet password is stored in this repository.

`cargo run -p argos-gpui` subscribes to the hosted fleet. Copy the example env file, set the password, and run from the repository root:

```sh
cp .env.example .env
cargo run -p argos-gpui
```

`.env` is gitignored. A missing file is fine: the window still opens, and a notice on the map says to set `ARGOS_ZENOH_PASSWORD` in `.env`. Variables already in the environment override `.env`. The loader reads `.env` from the workspace root and from the current directory.

`GOOGLE_MAPS_API_KEY` loads Photorealistic 3D Tiles. Put it in `.env` or the environment. The environment wins. The window loads that value before the map reads it, and it is never printed. Without a key the window still opens and a notice on the map says to set `GOOGLE_MAPS_API_KEY` in `.env`. The Google Cloud project needs billing and the Map Tiles API. For an offline check, `MAPS_DEMO=1` keeps the synthetic fixture and does not ask for a key:

```sh
MAPS_DEMO=1 cargo run -p argos-gpui
```

With a key:

```sh
export GOOGLE_MAPS_API_KEY='your-key'
cargo run -p argos-gpui
```

The window title is ARGOS. The console bar carries the map mode, **Map grid**, the latest status, the window's control claim, link health, and **Stop all**. Each rover has a callout on the map; rovers outside the view, and fleet members with no position, are tabs along the lower leading edge. Selecting a rover opens its inspector: state, trace, Take over / Drive / Stop, and Overview (hardware, autonomy, navigation, goal, and, while driving, the drive controls) and Video tabs. Empty-space drag still orbits the map in 3D; Shift-drag pans, scroll zooms, and 2 / 3 switch modes when the map controls are shown. Click a 3D rover or its ground marker to select it directly and activate this window's local control claim. The selected rover and marker turn green; this does not arm the rover or establish an exclusive remote ownership lease. Hold Control while dragging it to move the 3D model as a destination preview; its ground marker stays at the live position. No prior selection is needed. Dragging a pending goal also edits its destination. These gestures do not pan. Release sends the goal and restores the model to live telemetry. Esc, or dropping the ghost back on the rover, sends nothing. Map configuration, model conventions, and the standalone `learn-gpui` explorer are in [crates/terra-map/src/MAPS.md](crates/terra-map/src/MAPS.md).

The camera opens at a fallback location. It is not locked to a campus. Once one or more rovers have a position, the view recenters on their centroid, and it recenters again only when that set of rovers changes. Pose updates after that leave the camera where the operator put it. With no rover seen yet, the fallback is `ARGOS_MAP_START_LAT` / `ARGOS_MAP_START_LON`. Unset, those default to GMU Fairfax (38.8297, -77.3075).

| Variable | Default | Role |
| --- | --- | --- |
| `ARGOS_MAP_START_LAT`, `ARGOS_MAP_START_LON` | 38.8297, -77.3075 | Camera until a rover position arrives |
| `ARGOS_MAP_ORIGIN_LAT`, `ARGOS_MAP_ORIGIN_LON` | the start pair | Local-metre origin when Terra has not published a geographic alignment |
| `ARGOS_ZENOH_ENDPOINT` | `tls/argos.34.135.204.210.sslip.io:7448` | Hosted router. `tcp/` or `tls/` |
| `ARGOS_ZENOH_PREFIX` | `terra/phone` | Key prefix. Local Zorvane uses `terra/rover` |
| `ARGOS_ZENOH_USER` | `terra` | Bus user. Set it empty for a router with no authentication |
| `ARGOS_ZENOH_PASSWORD` | unset | No default and never committed. `.env` or the environment |
| `MAPS_DEMO` | unset | `1` draws the offline fixture and does not call Google |
| `GOOGLE_MAPS_API_KEY` | unset | Photorealistic 3D Tiles, from `.env` or the environment. The environment wins. Never committed or printed. Missing key: a map notice says to set it, unless `MAPS_DEMO=1` |
| `ARGOS_DRIVE_LINEAR` | 0.2 | Teleop speed cap, 0 through 0.5 m/s. Reverse uses the same cap |
| `ARGOS_DRIVE_YAW_RATE` | 0.5 | Yaw-rate cap, through 1 rad/s |
| `ARGOS_DRIVE_YAW_ACCEL` | 0.5 | How fast commanded yaw rate may rise, rad/s² |
| `ARGOS_DRIVE_YAW_DECEL` | 0.8 | How fast commanded yaw rate may fall, rad/s² |
| `ARGOS_DRIVE_MIN_RADIUS` | 0.5 | Minimum turn radius while rolling, metres. Spin off ignores in-place yaw |
| `ARGOS_DRIVE_ALLOW_SPIN` | off | `1`, `true`, or `yes` allows a turn with no forward speed |
| `ARGOS_DEGRADED_YAW_RATE` | 0.4 | Fallback yaw-rate warning when pose/quality is absent |
| `ARGOS_DEGRADED_SETTLE` | 1.0 | How long that fallback warning stays up, seconds |
| `ARGOS_FPV_STUN` | `stun:stun.l.google.com:19302` | STUN URLs, comma-separated. `none` keeps host candidates only |
| `ARGOS_FPV_TURN_URL` | unset | Optional `turn:` or `turns:` URL. Do not embed the username or password |
| `ARGOS_FPV_TURN_USERNAME` | unset | TURN username. Required together with the URL and credential |
| `ARGOS_FPV_TURN_CREDENTIAL` | unset | TURN credential. Never committed or printed |

FPV signaling comes from Terra's `fpv-signaling` crate (message types, JSON rules, topic keys, and the session machine). The dashboard uses `SignalingSession::dashboard()` and publishes only the effects that machine returns. Session ids are hyphenated UUIDs. `v` is the JSON number `1`. Signaling samples are capped at 65536 bytes. The dashboard subscribes before `start`. The machine stops a call that sits too long: 15 seconds for the offer, 20 for the answer, and 30 to connect. That timeout publishes `request` `stop`. A repeated `start` for a live call does not restart it. The phone offers H.264 first and VP8 as a fallback. No data channel is opened. Drive, arm, and stop stay on the existing Zenoh command path. The git dependency is pinned to branch `cursor/webrtc-fpv-ios-c3c2` until Terra PR 54 merges; point it at `main` after that.

A bad number, a half-set user/password pair on a `tcp/` router, or an endpoint that is not `tcp/` or `tls/` prints the reason and exits before the window opens. A hosted `tls/` router with no password does not exit: a map notice tells you to set `ARGOS_ZENOH_PASSWORD` in `.env`. A router that is down does not exit either. `MAPS_DEMO=1` keeps working in both cases.

Rover `x`/`y` are local metres. When the rover's localization report is version 1, mode `geographic`, and includes `originLatitude` / `originLongitude`, the marker uses that frame, including `anchorX`, `anchorY`, and `rotation`. Otherwise the marker uses `ARGOS_MAP_ORIGIN_*`. GPS fields on a local-mode report stay observations; they do not move the marker. Heading on the map is clockwise degrees. Terra yaw is radians from +x toward +y.

Markers are the `robot.glb` model at 12× scale and 80 m ellipsoid height so they stay visible at neighborhood zoom. That height is display exaggeration, not ground clamping. Online rovers with a pose newer than 2.5 s draw in the model color. Stale and offline rovers are tinted. The main map draws each rover's occupancy grid under it (see **Map grid** below). A cell at or above 65 is occupied; 0 through 64 are free; negative cells are unknown. The retired grid viewer cutoff of 50 is not used.

Select a rover. On the local navigation source, click its grid on the map (or drag its marker) to propose a destination, then confirm it in the popup. A GPS-placed rover takes local goals the same way: the click converts to local metres through the frame its grid is drawn in. A ghost follows the pointer, with the distance from the rover. Release publishes `{prefix}/{id}/goal` as `{"frame":"local","x","y","token"}` in metres, +x north and +y west. Local drags stay on L2 `waypoint_direct` or L3 `waypoint`, with a healthy link and a fresh pose that is not degraded.

Outdoor GPS is a separate navigation source. The inspector's navigation source toggle sends `{prefix}/{id}/navigation/source` and can change only while that rover is disarmed and has no active mission. Selecting a source does not arm the rover. After the vehicle has acknowledged `outdoor_gps`, drag its map marker. The ghost shows the arrival radius (5 m, or the radius in the outdoor report) and the GPS distance. Release publishes `{"frame":"wgs84","latitude","longitude","token"}` with those coordinates unchanged, and only from L3 `waypoint`. The radius is frozen at release; `run_id`, `navigation_revision`, and `arrival_radius_m` travel with the geographic goal so Terra validates the exact review. Esc, or dropping the ghost back on the rover, sends nothing. That release is the explicit operator action; it replaces the outdoor spec's separate confirm step. The drag stays disabled, with the admission reason, until the capability is acknowledged, the level is L3, and GPS, heading, and status are fresh. The desktop checks admission before publication and Terra validates the request against its current runtime state. Cancel goal still publishes `{"cancel":true}`. A rover with no version-1 outdoor capability shows **App update required**. The inspector keeps the raw GPS fix visible during a hold and names `GPS stale`, `Heading uncertain`, `Tracking lost`, `Obstacle map stale`, `Route blocked`, `Wheel feedback unavailable`, and `Paused` separately, along with the mission phase and remaining distance. The version-1 fields match Terra's outdoor controller. A paused mission exposes **Resume outdoor mission**; recovered sensors alone never resume motion. Source acceptance advances the navigation revision. The same local occupancy grid is drawn semi-transparently on the map under each rover, so there is no separate Local map tab. **Map grid** toggles that overlay. It uses the localization origin when there is one. A rover placed by its GPS fix gets the grid pinned at that fix and turned by the `heading_deg` in its outdoor report (Terra's fused heading), and the rover model turns the same way. Until a heading arrives the grid turns with the model's yaw instead. Those cells are only as well placed as the fix and heading, and are not claimed as geographic obstacles.

Autonomy buttons publish Terra's existing levels on `{prefix}/{id}/autonomy` as `{"level","token"}`: L0 `teleop`, L1 `assisted_teleop`, L2 `waypoint_direct`, L3 `waypoint`, L4 `explore`, plus `supervised` and `target_search`. L4 Explore is frontier exploration: Terra picks unmapped frontiers on its occupancy map and drives to them through its follower and local planner, with no per-goal approval. The request adds `budget_seconds` (3, 5 or 10 minutes from the **Exploration · L4** picker; ARGOS caps it at an hour). The inspector reads the `exploration` object nested on `autonomy/status` and shows the state, elapsed and remaining time, the current frontier target, area covered, frontiers tried, and why the run ended (no frontiers left, budget used, stop, takeover, stale map, safety hold). The target is also the rover's goal on the map. ARGOS does not send L4 while a stop is latched or while outdoor GPS is the navigation source, since Terra would end or hold the run at once. Stop, Cancel goal, or choosing another level ends a run. The inspector shows requested and effective levels. `assigned_level` is the recommendation when a status includes it. `heading_requires_spin_or_replan` is shown on the rover's callout, inspector, and goal marker. Reverse path segments are drawn dashed in amber when a route sample carries a negative velocity, `reverse`, or `direction`. A pair with no direction is marked provisional. `Reversing` appears when the reported forward speed is negative. S / Down already drives in reverse and uses the same turn-radius cap.

Drive mode publishes `{prefix}/{id}/teleop` at about 20 Hz only while a key is held: `{"linear","angular","operator_session_id","sequence","run_id","authority_revision"}`. WASD, arrows, and the on-screen pad share that path. The client cap defaults to 0.2 m/s, 0.5 rad/s yaw, 0.5 rad/s² accel, 0.8 rad/s² decel, and a 0.5 m turn radius, with in-place spin off. Those numbers match Terra's provisional turning policy and are defense in depth only. The payload does not claim an exemption. Space and Stop send the existing safety stop and a zero twist. Key release, focus loss, leaving drive, and switching rovers also send zero. Pose degradation does not block this teleop; it is informational. Phone Bluetooth manual is not given a VIO warning. New waypoint goals are refused while the pose is degraded. `<prefix>/{id}/pose/quality` (schema 1: `rover_id`, `run_id`, `sequence`, `degraded`, `reason`, `pose_age_s`, `imu_age_s`) drives that flag, including the same object nested on autonomy status. Until a quality message arrives, a yaw-rate heuristic is the fallback and is labeled that way. These quality and turning fields are provisional until Terra's turning-policy change lands.

Fleet membership still has to arrive on `<prefix>/fleet/state` before a pose is kept. Arm and Disarm sit on each rover card. They publish the existing `{prefix}/{id}/hardware` body Terra already admits: `{"action":"arm"|"disarm","token","run_id","authority_revision"}`. Disarm uses authority revision 0. Hold Arm for about one second. A click publishes nothing. Arm stays disabled, and the card shows the reason, unless that rover is selected, the link is healthy, the pose is fresh, safety is clear, no stop is latched, and this window holds control. **Take control** is the local 30-second operator lease, renewed while the window is open. It is the same gate as the old browser tab claim and is not a new Zenoh topic. Armed and disarmed are read from `{prefix}/{id}/hardware/status`. The card shows `arming…` until that status echoes the request token. After 10 seconds without that echo it shows `Hardware outcome unconfirmed; not retried. Check live hardware status.` and the rover's `reason` when one is present. A `result` that starts with `rejected` is shown as published, including a refusal such as missing wheel feedback. Disarm is one click and stays available. Space and Stop still latch the safety stop. **Reset** publishes the existing `{prefix}/{id}/safety` action `reset` and does not arm. Navigation source changes still require a disarmed rover. Choosing a source or a level does not arm. Teleop and goals show **Arm to drive** until hardware status says the rover is armed.

The built-in defaults are the hosted fleet: `tls/argos.34.135.204.210.sslip.io:7448`, prefix `terra/phone`, user `terra`. Take the password from the operator secret store (`/etc/argos/fleet.password` on the deployed host) and put it only in `.env` or the environment. `tls/` uses the system trust store.

Choose the connection for each dashboard launch:

```sh
cargo run -p argos-gpui -- --sim
cargo run -p argos-gpui -- --real
```

Simulation forces the local Zorvane endpoint and prefix, with no fleet credentials.
Start Zorvane separately first (`scripts/run-argos-local.sh` in its checkout).
Real mode uses your saved environment/`.env` fleet settings and is the default
when no argument is given. The window title identifies the selected mode.
Neither mode automatically starts Zorvane or combines local and remote fleets.
`--help` lists the options. The existing local launcher is a shortcut for sim mode.

For a separate local simulation session, start `scripts/run-argos-local.sh` in
an adjacent Zorvane checkout, then run `scripts/run-zorvane.sh` here. This leaves
the hosted `.env` settings intact and connects only this dashboard process to
the local simulator.

Local Zorvane listens on `tcp/127.0.0.1:7447` with prefix `terra/rover` and no user. Comment out the hosted lines in `.env` and uncomment the Zorvane block from `.env.example`, including the empty user and password lines so the hosted user is not kept:

```sh
ARGOS_ZENOH_ENDPOINT=tcp/127.0.0.1:7447
ARGOS_ZENOH_PREFIX=terra/rover
ARGOS_ZENOH_USER=
ARGOS_ZENOH_PASSWORD=
```

A standalone local development router is often `tcp/127.0.0.1:7448`.

`pi@terra1.local` runs the `terra-rover` BLE and motor service. That process does not publish Zenoh poses. Poses come from TerraPhone or from a Zenoh router on the LAN. If that host is running the router:

```sh
ARGOS_ZENOH_ENDPOINT=tcp/terra1.local:7447 \
ARGOS_ZENOH_PREFIX=terra/rover \
cargo run -p argos-gpui
```

## Fleet transport and hosting

The GPUI app connects directly through `argos-zenoh`. GCP runs only the
`argos-router` binary and Caddy for its public TLS certificate lifecycle.
See [GCP router setup and migration](packaging/gcp/README.md).
There is no HTTP application server, OAuth login, browser API or server-side audit service.

For a local development router:

```sh
docker compose up -d --build
```

This exposes unauthenticated TCP only on host loopback port 7448. Point local
clients to `tcp/127.0.0.1:7448` with no username or password. It does not join
GCP automatically. Stop it with `docker compose down`. For GCP, use the hosted
TLS endpoint and fleet credentials in `.env.example`.


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

Live transport tests need permission to bind loopback sockets. The Unity spike is unchanged and is not part of the dashboard.

## Workspace

| Crate | Role |
| --- | --- |
| `crates/argos-gpui` | Dashboard. Mounts `MapView`, subscribes to Zenoh, and publishes goals, autonomy, and teleop for the selected rover. |
| `crates/terra-map` | Map library (`terra_map`) and the `learn-gpui` explorer. |
| `crates/argos-zenoh` | Zenoh client, router binary, session log. |
| `crates/argos-core` | Shared contracts and operator state. |
| `crates/argos-ffi` | UniFFI facade. The Swift dashboard that called it is gone. |
| `spikes/unity-dashboard` | Historical prototype. |

### Change-driven visualization telemetry

Updated Terra and Zorvane publishers send full occupancy/cloud payloads when their content changes, with periodic recovery baselines for late subscribers. New live observations of unchanged content can instead send `map/occupancy/heartbeat` or `pointcloud/heartbeat`. These small messages identify the last full rover/run or frame/sequence; the Rust client renews only that matching cached visualization. Heartbeats never create missing data, authorize driving, or renew hardware/authority state. A stalled sensor emits neither fresh content nor heartbeats.

ARGOS retains compatibility with full-payload publishers. Deploy updated clients alongside change-driven publishers: older receivers that do not understand these heartbeats will mark unchanged maps stale between recovery baselines. Zorvane currently publishes occupancy and depth/RGB, not a separate point-cloud stream.

When updated Terra hardware reports `watchdog_paused`, the GPUI dashboard shows
**Armed · motion paused** and blocks drive/goal dispatch. Hold **Resume motion**
to request the existing safe-frame/ARM recovery handshake; the pending action
is not acknowledged until fresh hardware status clears the pause. Outdoor
mission Resume remains a separate explicit action. This requires the matching
TerraPhone and Pi service updates; the local simulator does not emulate Pi
command-timeout arming behavior.

### Manual control and stop reasons

For a single rover, select it, choose **Select manual control**, wait for L0 confirmation,
then hold **Arm** or **Resume motion** if required and choose **Drive this rover**.
Selecting manual control is permitted before hardware arm/resume; it never starts motion.
L0 does not require camera pose updates. Fresh fleet membership, control authority,
hardware status, command leases, and explicit emergency-stop recovery still apply.
Assisted driving and autonomous navigation retain their pose requirements.

The drive panel distinguishes missing/stale hardware status, arming, motion pause,
and disarmed hardware. Terra publishes its retained first cause as `stop_diagnostic`
on hardware status; ARGOS displays it for a stopped rover. Older phones fall back to
the reported fault/stop reason or a command-stream pause message. Both the Terra
phone runtime and ARGOS must be rebuilt for camera-independent fleet manual control.

Rust tests live under [`tests/`](tests/README.md), grouped by crate. Cargo test targets and private unit-module paths preserve their execution scopes.
