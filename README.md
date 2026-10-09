# ARGOS

ARGOS (Adaptive Robotic Group Operator System) is a native fleet operator app
for Terra, built with Rust and Swift. Terra owns sensing, onboard autonomy, and
low-level control; ARGOS owns fleet supervision and high-level goals.

Documentation: [https://super-yojan.dev/ARGOS/](https://super-yojan.dev/ARGOS/).
That site is the hub for ARGOS, [Terra](https://super-yojan.dev/Terra/),
[TerraPhone](https://super-yojan.dev/Terra/terraphone/), and
[Zorvane](https://super-yojan.dev/Zorvane/).

## Native iOS and macOS dashboard

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

### Hardware arming from ARGOS

Select the Terra rover in the 3D scene and use **Arm** in its panel. **Disarm** remains available during pending arming or missing authority telemetry. Terra may stay on its home screen; it forwards explicit requests to its paired rover and publishes confirmed hardware status. Driving and waypoint execution require fresh confirmed armed status. Waypoint drafting remains available while disarmed.

Arm requests are bound to the current rover run and authority revision and expire after 500 ms. Both endpoints and the final phone execution check enforce freshness. No reconnect, mode selection, or drive takeover automatically arms hardware. Existing background stops, Bluetooth acknowledgement, emergency-stop and rover watchdog protections remain in effect.
