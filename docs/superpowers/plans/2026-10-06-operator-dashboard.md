# ARGOS Apple Dashboard Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement task by task when implementation is requested. Track the steps below. This plan supersedes the Python web-dashboard proposal.

**Goal:** Native iOS and macOS operator apps showing the Terra fleet, live rover positions, waypoint progress, and send/cancel controls.

**Architecture:** A shared Rust library owns Zenoh, contract validation, telemetry, freshness, and command acknowledgement. UniFFI exposes this library to shared SwiftUI views and a Swift presentation model. Separate native iOS and macOS targets provide platform-specific navigation and configuration.

**Tech stack:** Rust, Zenoh, UniFFI, SwiftUI, MapKit, Cargo tests, XCTest/XCUITest. No Dioxus, Python dashboard runtime, browser bridge, or Node build.

**Spec:** https://github.com/Super-Yojan/ARGOS/issues/3 with user-requested replacement of the web MVP by iOS and macOS native apps. This changes the original browser/Codespaces acceptance scope; the Rust service tests can run in Codespaces, while Apple UI builds and simulator runs require macOS/Xcode.

## Design decisions

- Build in ARGOS, keeping Terra's sensing, autonomy, and motor control separate. Follow Terra's existing UniFFI static-library and Swift module conventions without importing its phone controller.
- Share Rust behavior and Swift views. Use a real macOS target and an iOS target; avoid a Catalyst-only desktop app.
- Proposed deployment floors: iOS 17 and macOS 14. Confirm build-tool availability before pinning dependencies or generating the Xcode project.
- Native Zenoh connects directly to a configured peer/router endpoint; multicast discovery is outside the first slice. Provide endpoint, topic prefix, local/geographic mode, and anchor settings.
- Default endpoint is tcp/127.0.0.1:7447 on Mac. On a physical iPhone, require the simulator computer's reachable LAN address: phone localhost is the phone. Terra must listen on an accessible interface. Document iOS local-network permission and macOS outgoing-network sandbox entitlement.
- Shared geographic map uses MapKit with default anchor 38.8297, -77.3075. Local practice-world mode uses a SwiftUI coordinate plot in metres. Map tap/click creates a draft target; an explicit Send control publishes it.
- Terra +x is north and +y is west in the anchored world. Goal-status x/y are target coordinates, never rover pose. Obtain pose from depth-frame body metadata, discarding pixels; receiving these packets still incurs depth bandwidth. Keep a future lightweight pose topic separate from MVP scope.
- Expose owned immutable snapshots through UniFFI, rather than Rust/Zenoh internals. Initial implementation polls snapshots at 5 Hz on a background Swift task; publish presentation changes on MainActor. This avoids callbacks crossing UI lifecycle boundaries.
- Rust connect/send/cancel work runs off the Swift main thread. A single connection owner coordinates all views/windows; disconnect is idempotent and releases subscriptions and worker tasks.
- Backgrounding/disconnection does not cancel Terra's latched goal. Suspend observation as appropriate, refresh after foreground/reconnect, and never replay a command. Do not promise continuous background iOS operation.
- Reuse the 2.5-second stale threshold with independent fleet, pose, and status receive ages. Missing telemetry remains unknown; fleet membership is not a per-rover heartbeat.
- Send each goal once with a generated UUID token. Show pending until a matching status confirms it; show unconfirmed after 5 seconds without acceptance. Do not retry automatically. Separate progress from acceptance and keep an active-but-stalled goal visible.
- Cancel publishes exactly {"cancel":true} once, then waits for a later idle status. No cancellation token exists; this is a single-operator MVP. Cancellation releases the goal but does not send a debug twist.
- Exclude video, urgency, natural-language commands, authentication, multi-user arbitration, and Android implementation. Keep the Rust API portable for later Kotlin bindings.
- Leave the existing Python CLI intact. Mirror the issue-required Python goal codecs in a small compatibility follow-up; all new native behavior and most tests use Rust/Swift.

## Planned files and interfaces

- Root Cargo.toml: ARGOS Rust workspace alongside the existing CLI.
- crates/argos-core/src/{contract,telemetry,state,lib}.rs: validated goal/status types, coordinate conversion, immutable snapshots, pending commands.
- crates/argos-zenoh/src/lib.rs: transport/session lifecycle and subscriptions; depends on core.
- crates/argos-ffi/src/lib.rs and uniffi.toml: exported ArgosClient and value records; depends on core and transport.
- crates/argos-ffi/src/bin/bindgen.rs: pinned UniFFI generation tool.
- scripts/build-apple.sh: Swift bindings and XCFramework packaging.
- apps/apple/ARGOS.xcodeproj: shared scheme and iOS/macOS targets.
- apps/apple/Shared/{FleetModel,FleetView,RoverDetailView,ConnectionView,WaypointEditor,GeographicMap,LocalMap}.swift.
- apps/apple/{iOS,macOS}: application entry points and platform configuration.
- apps/apple/Tests and UITests: Swift integration and user flows.
- README.md, docs/DESIGN.md, and CI workflows: native instructions and checks.

FFI surface: ArgosClient.connect(config), disconnect(), snapshot() -> FleetSnapshot, send_goal(rover_id, GoalRequest) -> CommandReceipt, cancel_goal(rover_id) -> CommandReceipt. Return typed errors for validation, unavailable member, disconnected transport, and publish failure. GoalRequest is a tagged local/geographic request, not an arbitrary JSON string. Snapshot includes membership, separate pose and goal records, ages, and local command phase/token. Serialize u64 IDs as native integer types throughout Swift/Rust.

## Review focus

1. A removed/stale rover cannot receive commands; gapped fleet IDs remain stable.
2. Old or mismatched tokens cannot acknowledge a new goal; goal IDs can reset on cancel or restart.
3. Map coordinates preserve north/west signs and separate destinations from live positions.
4. Disconnect/foreground transitions do not replay goals or leak subscriptions/tasks.
5. Rust errors and malformed telemetry cannot crash Swift or block the main thread.

## Task 1: Validate Apple transport and binding feasibility

- [ ] Inspect Terra's terra-mobile, terra-transport, build-ios.sh, and Swift smoke example. Choose compatible pinned UniFFI/Zenoh versions; reuse build conventions rather than copy onboard-control behavior.
- [ ] Add the minimal Rust workspace and an FFI client exposing connect, snapshot, and disconnect. Generate Swift bindings with shell/Cargo tooling.
- [ ] Package separate iOS-device, iOS-simulator, and macOS slices into an XCFramework; include Apple Silicon and Intel simulator/Mac support where toolchain dependencies permit. Never combine simulator and device libraries as one architecture slice.
- [ ] Run a small Swift smoke test on macOS and iOS Simulator that connects to Terra, receives real fleet state, and disconnects twice safely. Build a physical-device slice and record any signing/device-test limitation explicitly.
- [ ] Stop expanding the app if native iOS Zenoh linking or networking fails; resolve that evidence-backed blocker first. Commit the verified build slice.

## Task 2: Implement Rust contract and telemetry

- [ ] Write failing Cargo tests for exact local/WGS84/cancel JSON; custom prefixes; invalid IDs; finite numbers; local +/-20,000 m; latitude [-85,85]; longitude [-180,180]; yaw [-2pi,2pi]; 1–64 ASCII token characters [A-Za-z0-9._:-]; and 2048-byte command cap.
- [ ] Add status parsing tests for idle/active/arrived, optional fields, additive unknown fields, malformed/oversized payloads, and invalid states/numbers. State messages have a 65,536-byte cap.
- [ ] Add depth-header-only pose tests for missing body, key/header ID mismatch, invalid data, and binary pixel suffixes. Assert goal destination cannot become pose.
- [ ] Test origin mapping, north-positive latitude, west-negative longitude, and round-trip coordinate error under 1 mm near GMU using Terra's metres-per-degree constant 40,075,016.686 / 360.
- [ ] Implement the pure core APIs, run cargo test -p argos-core, and commit.

## Task 3: Implement sessions, snapshots, and goal tracking

- [ ] Add injectable clock and in-memory transport tests before behavior: discovery, gapped IDs, removal, independent freshness, invalid telemetry preserving last good receive age, and concurrent rover updates.
- [ ] Assert one publish per submission, generated tokens, pending -> active -> arrived, immediate arrived, mismatched tokens, 5-second unconfirmed state, cancel -> later idle, and no publish for stale/absent members.
- [ ] Test lower/reset goal IDs, transport loss, explicit reconnect, and idempotent shutdown. Fresh post-reconnect observations must replace historical state; unresolved submissions stay unconfirmed without replay.
- [ ] Implement bounded latest-state storage and subscriptions to fleet/state, */goal/status, and */camera/depth. No pixel decoding or forwarding. Reject telemetry for unknown membership.
- [ ] Expose owned records and typed errors through UniFFI. Verify generated Swift API, run Cargo tests plus a real localhost Zenoh integration test, and commit.

## Task 4: Build the shared SwiftUI operator flow

- [ ] Add FleetModel with connection lifecycle and 5-Hz background snapshot observation; apply UI changes on MainActor. Test cancellation of observation, errors, reconnect, and no main-thread network operations using a fake client.
- [ ] Add shared connection settings, fleet list, selected-rover details, goal input, draft target, Send, Cancel, remaining distance, token, and unknown/stale/unconfirmed states.
- [ ] Add MapKit geographic markers and a local coordinate plot. Validate both input modes and prevent map pan/tap from publishing a command by itself.
- [ ] Give macOS a sidebar/detail window with keyboard actions, and iOS navigation with a readable map/detail layout. Share feature views while adapting layout; add accessibility labels and non-color status text.
- [ ] Run XCTest and deterministic XCUITest flows: selection, waypoint draft/send, live pose, arrival, cancellation, malformed input, removed rover, disconnect, and foreground refresh. Commit.

## Task 5: Verify actual simulator behavior on both platforms

- [ ] Start one Terra rover with bundled tiles: TERRA_ROVER_COUNT=1 TERRA_TILES=1 TERRA_TILES_FETCH=0 cargo run from Terra/simulator. Verify tile activation and matching app anchor.
- [ ] On macOS and iOS Simulator, send latitude 38.82981, longitude -77.3075. Record actual movement, matching token, progress, and arrived; capture screenshots.
- [ ] Cancel a moving goal, observe idle, then verify an existing debug teleop command moves the rover. Test local-world waypoint input separately.
- [ ] Test physical-iPhone LAN connection if a signed device is available; explicitly distinguish simulator success from physical-device success. Test denial of local-network access and recovery.
- [ ] Test app background/foreground and simulator restart: clear stale presentation, obtain fresh state, and confirm zero command replay.
- [ ] Document Cargo/Xcode setup, binding generation, native launch, device endpoint configuration, permissions, world/anchor matching, cancellation semantics, and depth-bandwidth limitation. Attach macOS/iOS evidence to the eventual PR.

## Task 6: Preserve compatibility and CI

- [ ] Add the small Python TerraTopics.goal/goal_status, encode_goal, and status-parser compatibility change required by issue #3, with existing pytest contract tests. Keep it out of the app runtime and do not use Python build scripts.
- [ ] Run Rust tests on Linux, and Apple binding/build/Swift tests on a macOS runner. Test packaged XCFramework consumption rather than only source compilation.
- [ ] Keep the existing CLI suite passing. Update docs/DESIGN.md to describe the native supervision boundary and note the changed web/Codespaces scope.
- [ ] Run the full checks once at the final revision and map each acceptance requirement to automated or recorded native-simulator evidence.

## Completion criteria

Both native app targets build and run. Each can show at least one real simulated rover with independently fresh pose/status, send a correlated waypoint that reaches arrived, and cancel to release teleop. Rust/Swift tests pass, existing CLI compatibility is preserved, startup instructions reproduce the evidence, and macOS/iOS screenshots accompany the PR. Physical-device distribution, Android, and browser UI are not implied by successful native simulator builds.

## Sources

- UniFFI Xcode integration: https://mozilla.github.io/uniffi-rs/latest/swift/xcode.html
- UniFFI Swift bindings: https://mozilla.github.io/uniffi-rs/latest/swift/overview.html
- Apple multiplatform configuration: https://developer.apple.com/documentation/Xcode/configuring-a-multiplatform-app-target
- Local Terra integration reference: crates/terra-mobile and mobile/ios/TerraPhone.
