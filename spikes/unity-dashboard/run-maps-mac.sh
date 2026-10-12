#!/usr/bin/env bash
# GMU terrain is already in the player. This run requires live occupancy grids.
#
# On the Apple Silicon Mac, from the repo root:
#
#   ./spikes/unity-dashboard/run-maps-mac.sh
#
# Unity defaults to /Applications/Unity/Hub/Editor/6000.6.5f1/Unity.app.
# Override with UNITY_EDITOR set to that .app or to Contents/MacOS/Unity.
# Optional: ARGOS_ENDPOINT, ARGOS_PREFIX, ARGOS_ROVER_COUNT (default 8).
#
# The script builds the aarch64 dylib, builds a Mono player, starts the fake
# publisher (no Zorvane), and runs the player with -argosSource zenoh.
# It exits nonzero unless the log contains ARGOS occupancy grids: N for N >= 1
# and both overview.png and selected.png were written.
#
# Stop anything already bound to the endpoint (default tcp/127.0.0.1:7447).
#
# Zorvane, several rovers, no fake publisher. Do this instead of this script's
# publisher; the player arguments stay the same. From a Zorvane checkout:
#
#   TERRA_ROVER_COUNT=8 TERRA_TILES=1 TERRA_TILES_FETCH=0 cargo run -p zorvane
#
# Then run the built player:
#
#   spikes/unity-dashboard/builds/macos/ArgosFleetDashboard.app/Contents/MacOS/ArgosFleetDashboard \
#     -argosSource zenoh -argosEndpoint tcp/127.0.0.1:7447 \
#     -argosCaptureDir /tmp/argos-maps -argosQuit
#
# Zorvane publishes terra/rover/<id>/map/occupancy (schema 1). There is no merged
# fleet-map key yet (Terra #36). TERRA_ROVER_COUNT can be 15 to 20.
set -euo pipefail
export ARGOS_REQUIRE_MAPS=1
exec "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/run-zenoh-mac.sh"
