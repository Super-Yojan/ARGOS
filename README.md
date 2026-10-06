# ARGOS

ARGOS (Adaptive Robotic Group Operator System) supervises a Terra rover fleet over Zenoh.

Terra keeps local autonomy onboard: sensing, local planning, the watchdog, and low-level drive. ARGOS sends high-level commands — missions, goals, and fleet intents — and reads fleet health. That split is the operator contract. The first slice on the [ARGOS project board](https://github.com/users/Super-Yojan/projects/6) discovers rover ids, shows health, and can request a fleet size. `argos drive` publishes `cmd_vel` twists as a temporary debug path so the Zenoh session can be checked against today's simulator. It is not the long-term command interface.

Terra (https://github.com/Super-Yojan/Terra) owns the bus. Design, the target boundary, and the spike topic table: [docs/DESIGN.md](docs/DESIGN.md).

## Run against Terra

Start the simulator with its Zenoh bridge (default listen `tcp/127.0.0.1:7447`, prefix `terra/rover`):

```sh
cd /path/to/Terra/simulator
TERRA_ROVER_COUNT=3 cargo run
```

In another environment, install ARGOS and talk to that session:

```sh
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -e '.[dev]'

argos fleet
argos status
argos drive --rover 0 --linear 1.0 --angular 0.3 --seconds 5
argos watch
```

`drive` is the temporary debug twist: it repeats `cmd_vel` at 20 Hz, then sends zero. Terra stops a rover 500 ms after the last valid command. The long-term replacement is a mission or goal that Terra executes onboard. See [docs/DESIGN.md](docs/DESIGN.md).

From another machine, start Terra with `TERRA_ZENOH_LISTEN=tcp/0.0.0.0:7447` and point ARGOS at it:

```sh
argos status --endpoint tcp/SIMULATOR_IP:7447
```

If Terra was started with `TERRA_ZENOH_PREFIX`, pass the same value to `--prefix` (or set `ARGOS_ZENOH_PREFIX`). `ARGOS_ZENOH_ENDPOINT` overrides the default endpoint.

Request a fleet size only when you want Terra to spawn or remove rovers:

```sh
argos fleet --count 5
```

`status --json` and `fleet --json` print the same snapshot as a single JSON object.

## Run without the simulator

`mock_fleet` publishes `fleet/state` and logs twists. It is a smoke peer, not Terra: shrinking the fleet renumbers ids from zero instead of retiring them.

```sh
python -m argos.mock_fleet --listen tcp/127.0.0.1:7447 --count 3
```

Leave that process running, then use the `argos` commands above.

## Tests

```sh
python -m pytest
```

Unit tests use an in-memory transport. One test opens a real Zenoh peer and client on localhost, still without Terra.

## Layout

| Path | Role |
| --- | --- |
| `src/argos/contract.py` | Topic names and JSON codecs. Change these if Terra's keys move. |
| `src/argos/supervisor.py` | Discover, resize, and drive |
| `src/argos/health.py` | Link and motion health derived from fleet state |
| `src/argos/cli.py` | `status`, `watch`, `fleet`, `drive` |
| `src/argos/mock_fleet.py` | Local stand-in for Terra's three supervision keys |

## Native iOS and macOS dashboard

The Apple dashboard uses a shared Rust Zenoh client through UniFFI and native
SwiftUI/MapKit views. Python is not part of the app runtime. The existing CLI
continues to work independently.

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

Select `ARGOSMac` for a native Mac app or `ARGOSiOS` for iPhone/iPad. Device builds
need your Apple signing team. The macOS target has outgoing-network sandbox
permission; iOS describes its local-network access request. Allow local-network
access when prompted.

Start the Terra simulator in another terminal. A geographic demo using its
bundled GMU anchor is:

```sh
cd /path/to/Terra/simulator
TERRA_ROVER_COUNT=1 TERRA_TILES=1 TERRA_TILES_FETCH=0 cargo run
```

In ARGOS, open **Connection**, set endpoint `tcp/127.0.0.1:7447`, prefix
`terra/rover`, and enable **Geographic map**. Keep latitude `38.8297`, longitude
`-77.3075` to match that simulator. Select a rover, tap/click a target or enter
latitude `38.82981`, longitude `-77.3075`, then choose **Send waypoint**. The app
shows the correlation token, acceptance, distance remaining, and arrival.
For the default flat practice world, disable Geographic map and enter local
x/y coordinates in metres; +x is north/forward, +y is west/left. The local plot
supports zooming without an internet basemap.

A physical phone needs your simulator computer's LAN address, not phone
localhost. Start Terra with `TERRA_ZENOH_LISTEN=tcp/0.0.0.0:7447` and enter
`tcp/COMPUTER_LAN_IP:7447` in the app. The app's configured anchor must match
Terra; the bus does not advertise the anchor or whether tile loading succeeded.

**Send means published, not accepted.** A matching Terra status confirms the
UUID token. After five seconds without confirmation the app shows unconfirmed;
it does not resend. Cancel sends one cancellation and waits for idle. This
releases the latched goal for debug teleop; it does not send a teleop command.
Closing, disconnecting, or backgrounding the app does not cancel Terra's goal.
Observation resumes with fresh receive ages; commands are never replayed by ARGOS.

Pose comes from exposure-aligned `body` metadata in depth packets. The app
receives depth packets but discards their pixels, so depth-network bandwidth is
still present. Pose, goal status, and fleet membership have independent ages;
2.5 seconds is stale. Missing pose stays unknown. Terra goal-status x/y denote
the destination. MapKit basemap imagery needs network access; local controls do
not. Terra's waypoint follower does not plan around obstacles.

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
Xcode on macOS. There is no browser dashboard in this native slice.
