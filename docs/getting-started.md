# Getting started

ARGOS is a Rust workspace (`argos-core`, `argos-zenoh`, `argos-ffi`) plus an Xcode project for macOS and iOS. Rust tests run on Linux. The native apps need macOS with Xcode. Deployment floors are macOS 14 and iOS 17. The committed slices are Apple Silicon Mac, physical iOS devices, and Apple Silicon iOS simulators.

## Rust

Edition 2024, so Rust 1.85 or newer. From this repo:

```sh
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

`cargo test` covers contract validation, fleet state, occupancy, operator actions, the Zenoh loopback, and the UniFFI surface. Those tests open an in-process Zenoh peer. They do not need Zorvane.

A live mission smoke against an isolated listener:

```sh
cargo run -p argos-zenoh --example mission_smoke -- tcp/127.0.0.1:7448
```

Start the listener on `tcp/127.0.0.1:7448` first. With Zorvane that is a mission world bound to that port (see below). The example requests each autonomy level, sends a waypoint, approves a supervised proposal when one is offered, then fleet-takeover and fleet-stop. It writes a JSONL session under `/private/tmp` on macOS. Run it alone; the native UI test uses the same port.

## Apple app

```sh
rustup target add aarch64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim
gem install xcodeproj
./scripts/build-apple.sh
open apps/apple/ARGOS.xcodeproj
```

`scripts/build-apple.sh` builds `argos-ffi`, generates Swift bindings with UniFFI, and packs an XCFramework for the three Apple Silicon slices. The Xcode project is committed. Regenerate it with `ruby scripts/create-apple-project.rb` only when sources are added or removed.

Select `ARGOSMac` or `ARGOSiOS`. Device builds need an Apple signing team. The Mac target has an outgoing-network entitlement. iOS asks for local-network access; allow it when prompted.

```sh
xcodebuild -project apps/apple/ARGOS.xcodeproj -scheme ARGOSMac \
  -destination 'platform=macOS,arch=arm64' -derivedDataPath apps/apple/build \
  test -only-testing:ARGOSMacTests CODE_SIGNING_ALLOWED=NO
xcodebuild -project apps/apple/ARGOS.xcodeproj -scheme ARGOSiOS \
  -destination 'platform=iOS Simulator,name=iPhone 18 Pro' \
  -derivedDataPath apps/apple/build-ios test -only-testing:ARGOSiOSTests \
  CODE_SIGNING_ALLOWED=NO
```

Use an installed simulator name. `./scripts/check-swift.sh` drives a real arrival and cancel through the generated bindings and needs Zorvane already running in the geographic demo. UI tests (`ARGOSMacUITests`, `ARGOSiOSUITests`) need that same live world. Do not run both live command tests at once.

## Run against Zorvane

[Zorvane](https://super-yojan.dev/Zorvane/) is the world simulator extracted from Terra. The Zenoh prefix stays `terra/rover`. The default peer listens on `tcp/127.0.0.1:7447` with multicast discovery off, which matches ARGOS’s client.

Clone Zorvane with Git LFS (`rover.glb` is about 118 MB). Rust edition 2024 and the Bevy window libraries are listed in the Zorvane README. Then, for the geographic demo whose anchor is the bundled GMU Johnson Center patch:

```sh
cd /path/to/Zorvane
TERRA_ROVER_COUNT=1 TERRA_TILES=1 TERRA_TILES_FETCH=0 cargo run -p zorvane
```

In ARGOS, open **Connection**:

| Field | Value |
| --- | --- |
| Endpoint | `tcp/127.0.0.1:7447` |
| Prefix | `terra/rover` |
| Geographic map | on |
| Anchor | latitude `38.8297`, longitude `-77.3075` |

Select a rover, choose **Waypoint**, and send latitude `38.82981`, longitude `-77.3075` (about 12 m north of that anchor). The app shows the token, then acceptance, distance remaining, and arrival when `goal/status` echoes the token with `state: arrived`. Cancel and wait for `idle`.

For the flat practice world, leave geographic mode off and type local x/y in metres. +x is north/forward, +y is west/left. Spawn is the origin.

A mission world, for the smoke example or the autonomy panel, adds `TERRA_MISSION=1`. Point the smoke example at a dedicated port so it does not share the interactive listener:

```sh
cd /path/to/Zorvane
TERRA_MISSION=1 TERRA_ZENOH_LISTEN=tcp/127.0.0.1:7448 cargo run -p zorvane
```

`TERRA_ROVER_COUNT` sets the fleet from 0 through 32. `ZORVANE_VEHICLE` selects the body and defaults to `terra-ground`. Tile and world variables are documented on the Zorvane site.

The anchor and the tile set are operator configuration. The bus does not advertise them, and it does not say whether tile loading succeeded. A mismatch puts goals on the wrong patch.

## Docs site

The pages under `docs/` are the MkDocs Material site. From this repo:

```sh
python3 -m pip install -r requirements-docs.txt
mkdocs serve
```

`mkdocs build --strict` is what CI runs. The published hub is [https://super-yojan.dev/ARGOS/](https://super-yojan.dev/ARGOS/).
