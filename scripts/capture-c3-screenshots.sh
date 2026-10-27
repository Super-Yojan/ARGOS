#!/bin/sh
# Capture the current argos-gpui window against the loopback screenshot fixture.
#
# Requires a built binary, a Vulkan driver (lavapipe is enough), and Xvfb or
# an existing DISPLAY. Writes PNGs under docs/screenshots/.
#
#   scripts/capture-c3-screenshots.sh up
#   scripts/capture-c3-screenshots.sh shot docs/screenshots/c3-console/01-overview-3d.png
#   scripts/capture-c3-screenshots.sh down
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
display=${ARGOS_CAPTURE_DISPLAY:-:99}
export DISPLAY="$display"
export MAPS_DEMO=1
export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
runtime=${ARGOS_CAPTURE_RUNTIME:-/tmp/argos-capture}
mkdir -p "$runtime"

bin="$root/target/debug/argos-gpui"
fixture="$root/target/debug/examples/screenshot-fixture"

up() {
  if ! [ -x "$bin" ] || ! [ -x "$fixture" ]; then
    echo "build first: cargo build -p argos-gpui --bin argos-gpui --example screenshot-fixture" >&2
    exit 1
  fi
  if ! xdpyinfo -display "$display" >/dev/null 2>&1; then
    Xvfb "$display" -screen 0 1600x1000x24 +extension GLX +render -noreset >"$runtime/xvfb.log" 2>&1 &
    echo $! >"$runtime/xvfb.pid"
    i=0
    while ! xdpyinfo -display "$display" >/dev/null 2>&1; do
      i=$((i + 1))
      if [ "$i" -gt 50 ]; then
        echo "Xvfb did not start" >&2
        exit 1
      fi
      sleep 0.1
    done
  fi
  if ! [ -f "$runtime/fixture.pid" ] || ! kill -0 "$(cat "$runtime/fixture.pid")" 2>/dev/null; then
    "$fixture" >"$runtime/fixture.log" 2>&1 &
    echo $! >"$runtime/fixture.pid"
    i=0
    while ! grep -q "screenshot fixture listening" "$runtime/fixture.log" 2>/dev/null; do
      i=$((i + 1))
      if [ "$i" -gt 50 ]; then
        echo "fixture did not listen; see $runtime/fixture.log" >&2
        exit 1
      fi
      sleep 0.1
    done
  fi
  if ! [ -f "$runtime/app.pid" ] || ! kill -0 "$(cat "$runtime/app.pid")" 2>/dev/null; then
    "$bin" --sim >"$runtime/app.log" 2>&1 &
    echo $! >"$runtime/app.pid"
  fi
  echo "fixture $(cat "$runtime/fixture.pid") app $(cat "$runtime/app.pid") display $display"
}

shot() {
  out=$1
  mkdir -p "$(dirname "$out")"
  wid=$(xdotool search --name "ARGOS" | head -n 1)
  if [ -z "$wid" ]; then
    echo "ARGOS window not found" >&2
    exit 1
  fi
  # A window manager puts the console bar in the parent frame. The named
  # client window starts below that bar, so grab the parent when it is larger.
  parent=$(xwininfo -id "$wid" -tree 2>/dev/null | awk '/Parent/ {print $4; exit}')
  target=$wid
  if [ -n "${parent:-}" ]; then
    eval "$(xdotool getwindowgeometry --shell "$parent")"
    parent_h=$HEIGHT
    eval "$(xdotool getwindowgeometry --shell "$wid")"
    # The frame is a few dozen pixels taller. The root window is not.
    extra=$((parent_h - HEIGHT))
    if [ "$extra" -gt 0 ] && [ "$extra" -lt 80 ]; then
      target=$parent
    fi
  fi
  eval "$(xdotool getwindowgeometry --shell "$target")"
  ffmpeg -y -loglevel error -f x11grab -video_size "${WIDTH}x${HEIGHT}" -i "${display}.0+${X},${Y}" -frames:v 1 "$out"
  echo "$out ${WIDTH}x${HEIGHT} at ${X},${Y}"
}

down() {
  if [ -f "$runtime/app.pid" ]; then
    kill "$(cat "$runtime/app.pid")" 2>/dev/null || true
    rm -f "$runtime/app.pid"
  fi
  if [ -f "$runtime/fixture.pid" ]; then
    kill "$(cat "$runtime/fixture.pid")" 2>/dev/null || true
    rm -f "$runtime/fixture.pid"
  fi
  if [ -f "$runtime/xvfb.pid" ]; then
    kill "$(cat "$runtime/xvfb.pid")" 2>/dev/null || true
    rm -f "$runtime/xvfb.pid"
  fi
}

cmd=${1:-}
case "$cmd" in
  up) up ;;
  shot)
    shift
    shot "${1:?png path}"
    ;;
  down) down ;;
  *)
    echo "usage: $0 up | shot <png> | down" >&2
    exit 2
    ;;
esac
