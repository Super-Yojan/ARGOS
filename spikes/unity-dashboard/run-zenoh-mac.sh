#!/usr/bin/env bash
# Prove the Unity Mac player can see fake rovers through the Rust Zenoh bridge.
#
# Run this on the Apple Silicon Mac (the cloud VM has no Unity license):
#
#   ./spikes/unity-dashboard/run-zenoh-mac.sh
#
# Unity defaults to /Applications/Unity/Hub/Editor/6000.6.5f1/Unity.app.
# Override with UNITY_EDITOR set to that .app bundle or to Contents/MacOS/Unity.
#
# What it does, in order:
#   1. Builds libargos_unity_bridge.dylib for aarch64-apple-darwin.
#   2. Copies it to Assets/Plugins/macOS and writes the ARM64 plugin import settings.
#   3. Builds a Mono Mac player with Unity (IL2CPP is not required).
#   4. Starts the fake fleet publisher on tcp/127.0.0.1:7447 (no Zorvane).
#   5. Runs the player with -argosSource zenoh -argosQuit.
#   6. Exits nonzero unless the player log reports at least one live rover.
#
# Stop Zorvane first if it is already bound to port 7447.
#
# Terrain plus occupancy (fails unless a grid arrives, and unless both captures exist):
#   ./spikes/unity-dashboard/run-maps-mac.sh
#
# Zorvane, instead of the fake publisher (not what this script does):
#   TERRA_ROVER_COUNT=8 TERRA_TILES=1 TERRA_TILES_FETCH=0 cargo run -p zorvane
#   then run the player yourself:
#   ArgosFleetDashboard.app/Contents/MacOS/ArgosFleetDashboard \
#     -argosSource zenoh -argosEndpoint tcp/127.0.0.1:7447 \
#     -argosCaptureDir /tmp/argos-zenoh -argosQuit
#
# Knobs: UNITY_EDITOR, ARGOS_ENDPOINT, ARGOS_ROVER_COUNT, ARGOS_PREFIX.
set -euo pipefail

SPIKE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SPIKE/../.." && pwd)"
UNITY_APP="${UNITY_EDITOR:-/Applications/Unity/Hub/Editor/6000.6.5f1/Unity.app}"
ENDPOINT="${ARGOS_ENDPOINT:-tcp/127.0.0.1:7447}"
PREFIX="${ARGOS_PREFIX:-terra/rover}"
COUNT="${ARGOS_ROVER_COUNT:-8}"
ARTIFACTS="$SPIKE/artifacts/zenoh-mac"
PLAYER_APP="$SPIKE/builds/macos/ArgosFleetDashboard.app"
CAPTURE="$ARTIFACTS/capture"
PLUGIN_DIR="$SPIKE/Assets/Plugins/macOS"
TARGET="aarch64-apple-darwin"

mkdir -p "$ARTIFACTS" "$CAPTURE" "$PLUGIN_DIR"
: > "$ARTIFACTS/run.log"

log() { printf '%s\n' "$*" | tee -a "$ARTIFACTS/run.log"; }

if [[ -d "$UNITY_APP" ]]; then
  UNITY_BIN="$UNITY_APP/Contents/MacOS/Unity"
else
  UNITY_BIN="$UNITY_APP"
fi

PUB_PID=""
PLAYER_PID=""
cleanup() {
  if [[ -n "$PLAYER_PID" ]] && kill -0 "$PLAYER_PID" 2>/dev/null; then
    log "stopping player pid $PLAYER_PID"
    kill "$PLAYER_PID" 2>/dev/null || true
    wait "$PLAYER_PID" 2>/dev/null || true
  fi
  if [[ -n "$PUB_PID" ]] && kill -0 "$PUB_PID" 2>/dev/null; then
    log "stopping publisher pid $PUB_PID"
    kill "$PUB_PID" 2>/dev/null || true
    wait "$PUB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

log "ARGOS zenoh Mac run"
log "repo: $ROOT"
log "unity: $UNITY_BIN"
log "endpoint: $ENDPOINT  prefix: $PREFIX  rovers: $COUNT"
log "capture: $CAPTURE"

if ! command -v cargo >/dev/null 2>&1; then
  log "FAIL: cargo is not on PATH. Install rustup, then rerun this script."
  exit 1
fi
if [[ ! -x "$UNITY_BIN" ]]; then
  log "FAIL: Unity binary is not executable: $UNITY_BIN"
  log "Set UNITY_EDITOR to /Applications/Unity/Hub/Editor/6000.6.5f1/Unity.app or to Contents/MacOS/Unity."
  exit 1
fi

log "step 1/5: rust target $TARGET"
if command -v rustup >/dev/null 2>&1; then
  if ! rustup target list --installed | grep -qx "$TARGET"; then
    log "installing rustup target $TARGET"
    rustup target add "$TARGET"
  fi
  log "cargo build --release --target $TARGET -p argos-unity-bridge"
  (cd "$ROOT" && cargo build --release --target "$TARGET" -p argos-unity-bridge)
  DYLIB="$ROOT/target/$TARGET/release/libargos_unity_bridge.dylib"
else
  host="$(uname -m)"
  if [[ "$host" != "arm64" ]]; then
    log "FAIL: rustup is required to build $TARGET on $host."
    exit 1
  fi
  log "rustup is not installed. This Mac is arm64, so the host release dylib is $TARGET."
  log "cargo build --release -p argos-unity-bridge"
  (cd "$ROOT" && cargo build --release -p argos-unity-bridge)
  DYLIB="$ROOT/target/release/libargos_unity_bridge.dylib"
fi

if [[ ! -f "$DYLIB" ]]; then
  log "FAIL: dylib was not produced at $DYLIB"
  exit 1
fi
log "dylib: $DYLIB ($(wc -c < "$DYLIB" | tr -d ' ') bytes)"

log "step 2/5: copy plugin into Assets/Plugins/macOS"
cp -f "$DYLIB" "$PLUGIN_DIR/libargos_unity_bridge.dylib"
cat > "$PLUGIN_DIR/libargos_unity_bridge.dylib.meta" <<'EOF'
fileFormatVersion: 2
guid: ebf7235f81937e62fa2405163dce8b79
PluginImporter:
  externalObjects: {}
  serializedVersion: 2
  iconMap: {}
  executionOrder: {}
  defineConstraints: []
  isPreloaded: 0
  isOverridable: 1
  isExplicitlyReferenced: 0
  validateReferences: 1
  platformData:
  - first:
      Any: 
    second:
      enabled: 0
      settings: {}
  - first:
      Editor: Editor
    second:
      enabled: 1
      settings:
        CPU: ARM64
        DefaultValueInitialized: true
        OS: OSX
  - first:
      Standalone: Linux64
    second:
      enabled: 0
      settings:
        CPU: None
  - first:
      Standalone: OSXUniversal
    second:
      enabled: 1
      settings:
        CPU: ARM64
  - first:
      Standalone: Win
    second:
      enabled: 0
      settings:
        CPU: None
  - first:
      Standalone: Win64
    second:
      enabled: 0
      settings:
        CPU: None
  userData: 
  assetBundleName: 
  assetBundleVariant: 
EOF
log "plugin import: Standalone OSXUniversal CPU ARM64, Editor OSX ARM64"

log "step 3/5: Unity Mono player (this editor needs a Personal license already activated in Hub)"
log "prepare log: $ARTIFACTS/unity-prepare.log"
if ! "$UNITY_BIN" \
  -batchmode -nographics -quit \
  -projectPath "$SPIKE" \
  -executeMethod Argos.Fleet.EditorTools.FleetSceneSetup.PrepareMacPlayer \
  -logFile "$ARTIFACTS/unity-prepare.log"; then
  log "FAIL: PrepareMacPlayer failed. A missing license exits 198. See $ARTIFACTS/unity-prepare.log"
  exit 1
fi
log "prepare exited 0. Building the player. Log: $ARTIFACTS/unity-build.log"
if ! "$UNITY_BIN" \
  -batchmode -nographics -quit \
  -projectPath "$SPIKE" \
  -buildOSXUniversalPlayer "$PLAYER_APP" \
  -logFile "$ARTIFACTS/unity-build.log"; then
  log "FAIL: Mac player build failed. See $ARTIFACTS/unity-build.log"
  exit 1
fi
PLAYER_BIN="$PLAYER_APP/Contents/MacOS/ArgosFleetDashboard"
if [[ ! -x "$PLAYER_BIN" ]]; then
  log "FAIL: player binary missing at $PLAYER_BIN"
  log "See $ARTIFACTS/unity-build.log"
  exit 1
fi
log "player: $PLAYER_BIN ($(du -sh "$PLAYER_APP" | awk '{print $1}'))"

log "step 4/5: fake publisher (listener, no Zorvane)"
PUBLISHER="$ROOT/target/$TARGET/release/argos-fleet-publisher"
if [[ ! -x "$PUBLISHER" ]]; then
  PUBLISHER="$ROOT/target/release/argos-fleet-publisher"
fi
"$PUBLISHER" --count "$COUNT" --endpoint "$ENDPOINT" --prefix "$PREFIX" --listen \
  > "$ARTIFACTS/publisher.log" 2>&1 &
PUB_PID=$!
log "publisher pid $PUB_PID, log $ARTIFACTS/publisher.log"
ready=0
for _ in $(seq 1 50); do
  if grep -q "ARGOS publisher ready" "$ARTIFACTS/publisher.log"; then
    ready=1
    break
  fi
  if ! kill -0 "$PUB_PID" 2>/dev/null; then
    break
  fi
  sleep 0.2
done
if [[ "$ready" != 1 ]]; then
  log "FAIL: publisher did not become ready. Port ${ENDPOINT} may already be taken (stop Zorvane) ."
  log "----- publisher.log -----"
  cat "$ARTIFACTS/publisher.log" | tee -a "$ARTIFACTS/run.log"
  exit 1
fi
log "publisher ready"

log "step 5/5: player -argosSource zenoh -argosEndpoint $ENDPOINT -argosQuit"
rm -f "$CAPTURE/overview.png" "$CAPTURE/selected.png" "$CAPTURE/presentation.txt"
"$PLAYER_BIN" \
  -argosSource zenoh \
  -argosEndpoint "$ENDPOINT" \
  -argosPrefix "$PREFIX" \
  -argosCaptureDir "$CAPTURE" \
  -argosQuit \
  -logFile "$ARTIFACTS/player.log" &
PLAYER_PID=$!
log "player pid $PLAYER_PID, log $ARTIFACTS/player.log"
for _ in $(seq 1 90); do
  if ! kill -0 "$PLAYER_PID" 2>/dev/null; then
    break
  fi
  sleep 1
done
if kill -0 "$PLAYER_PID" 2>/dev/null; then
  log "FAIL: player still running after 90s"
  exit 1
fi
set +e
wait "$PLAYER_PID"
player_exit=$?
set -e
PLAYER_PID=""
log "player exit $player_exit"

live_line="$(grep -E 'ARGOS live rovers: [0-9]+' "$ARTIFACTS/player.log" | tail -1 || true)"
live_count="$(printf '%s\n' "$live_line" | sed -n 's/.*ARGOS live rovers: \([0-9][0-9]*\).*/\1/p')"
log "player says: ${live_line:-<no ARGOS live rovers line>}"
if [[ -f "$CAPTURE/presentation.txt" ]]; then
  log "presentation.txt:"
  cat "$CAPTURE/presentation.txt" | tee -a "$ARTIFACTS/run.log"
else
  log "presentation.txt was not written"
fi
if [[ -f "$CAPTURE/overview.png" ]]; then
  log "overview.png: $(wc -c < "$CAPTURE/overview.png" | tr -d ' ') bytes"
fi

if [[ -z "${live_count:-}" || "$live_count" -lt 1 ]]; then
  log "FAIL: no live rovers were received."
  log "Look at $ARTIFACTS/player.log and $ARTIFACTS/publisher.log"
  exit 1
fi
if [[ ! -f "$CAPTURE/presentation.txt" ]] || ! grep -q 'terra-' "$CAPTURE/presentation.txt"; then
  log "FAIL: player log counted rovers, but presentation.txt has no terra- rover."
  exit 1
fi

if [[ "${ARGOS_REQUIRE_MAPS:-}" == 1 ]]; then
  grid_line="$(grep -E 'ARGOS occupancy grids: [0-9]+' "$ARTIFACTS/player.log" | tail -1 || true)"
  grid_count="$(printf '%s\n' "$grid_line" | sed -n 's/.*ARGOS occupancy grids: \([0-9][0-9]*\).*/\1/p')"
  log "player says: ${grid_line:-<no ARGOS occupancy grids line>}"
  overview_bytes=0
  selected_bytes=0
  if [[ -f "$CAPTURE/overview.png" ]]; then
    overview_bytes="$(wc -c < "$CAPTURE/overview.png" | tr -d ' ')"
  fi
  if [[ -f "$CAPTURE/selected.png" ]]; then
    selected_bytes="$(wc -c < "$CAPTURE/selected.png" | tr -d ' ')"
  fi
  log "overview.png: ${overview_bytes} bytes"
  log "selected.png: ${selected_bytes} bytes"
  if [[ -z "${grid_count:-}" || "$grid_count" -lt 1 ]]; then
    log "FAIL: no occupancy grid was received."
    log "Look at $ARTIFACTS/player.log and $ARTIFACTS/publisher.log"
    exit 1
  fi
  if [[ "$overview_bytes" -lt 1000 || "$selected_bytes" -lt 1000 ]]; then
    log "FAIL: overview.png and selected.png were not both captured."
    exit 1
  fi
  log "PASS: live rovers $live_count, occupancy grids $grid_count"
  log "captures: $CAPTURE"
  exit 0
fi

log "PASS: live rovers received: $live_count"
log "captures: $CAPTURE"
exit 0
