# Getting started

!!! tip "TL;DR"
    Rust tests run anywhere.
    The app needs a Mac with Xcode.
    Start Zorvane on `tcp/127.0.0.1:7447`.
    In ARGOS set prefix `terra/rover` and the GMU anchor `38.8297, -77.3075`.

<div class="shot-row" markdown="1">

<figure class="wide" markdown="1">
![Connected Mac dashboard with rover 8 and an active goal.](assets/macos-dashboard.jpg)
<figcaption>What “it works” looks like. A rover on the map. A goal ARGOS has accepted.</figcaption>
</figure>

<figure class="phone" markdown="1">
![iPhone simulator showing the same fleet and a cancelled goal.](assets/ios-dashboard.jpg)
<figcaption>Same session shape on the iOS simulator.</figcaption>
</figure>

</div>

## Zorvane, then ARGOS

[Zorvane](https://super-yojan.dev/Zorvane/) is the world. Clone it with Git LFS.

```sh
cd /path/to/Zorvane
TERRA_ROVER_COUNT=1 TERRA_TILES=1 TERRA_TILES_FETCH=0 cargo run -p zorvane
```

ARGOS → **Connection**:

| Field | Value |
| --- | --- |
| Endpoint | `tcp/127.0.0.1:7447` |
| Prefix | `terra/rover` |
| Geographic map | on |
| Anchor | `38.8297`, `-77.3075` |

Select the rover. Choose **Waypoint**. Send `38.82981, -77.3075`.

You should see the token, then active, then arrived.

Cancel. Wait for idle.

Flat world: turn geographic mode off. Type local metres. +x is north. +y is west.

The bus does not tell you the anchor. If the numbers disagree, the goal lands on the wrong patch.

## Mission smoke

Use a second port so it stays off the interactive sim.

```sh
# Zorvane
TERRA_MISSION=1 TERRA_ZENOH_LISTEN=tcp/127.0.0.1:7448 cargo run -p zorvane

# ARGOS
cargo run -p argos-zenoh --example mission_smoke -- tcp/127.0.0.1:7448
```

The example walks autonomy levels, a waypoint, a supervised approval, fleet takeover, and fleet stop.

Run it alone. The UI test wants that same port.

## Rust checks

Edition 2024 (Rust 1.85+). No simulator required.

```sh
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

## Apple app

macOS 14+. iOS 17+. Apple Silicon slices: Mac, device, simulator.

```sh
rustup target add aarch64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim
gem install xcodeproj
./scripts/build-apple.sh
open apps/apple/ARGOS.xcodeproj
```

`ARGOSMac` or `ARGOSiOS`. A device build needs your signing team.

Allow local network on the phone. The Mac target already has outgoing network.

```sh
./scripts/check-swift.sh
```

That script needs the geographic Zorvane demo already running.

Regenerate the Xcode project only when sources change: `ruby scripts/create-apple-project.rb`.

## This site

```sh
python3 -m pip install -r requirements-docs.txt
mkdocs serve
```

CI runs `mkdocs build --strict`. Published at [super-yojan.dev/ARGOS](https://super-yojan.dev/ARGOS/).
