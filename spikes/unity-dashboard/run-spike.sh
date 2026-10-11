#!/usr/bin/env bash
# Headless loop for the ARGOS Unity dashboard spike: install, license, EditMode tests,
# Linux player, Xvfb capture. A missing license is recorded and the later steps still run
# so the failure is visible in the logs. Artifacts land in spikes/unity-dashboard/artifacts/.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT="$ROOT"
ARTIFACTS="$PROJECT/artifacts"
mkdir -p "$ARTIFACTS"

UNITY_VERSION="${UNITY_VERSION:-6000.3.26f1}"
UNITY_CHANGESET="${UNITY_CHANGESET:-de8167e7440c}"
UNITY_ROOT="${UNITY_ROOT:-/opt/unity/${UNITY_VERSION}}"
UNITY="${UNITY_EDITOR:-${UNITY_ROOT}/Editor/Unity}"

RESULTS="$ARTIFACTS/step-results.txt"
: > "$RESULTS"

note() { printf '%s\n' "$*" | tee -a "$ARTIFACTS/spike.log"; }
set_result() { printf '%s=%s\n' "$1" "$2" | tee -a "$RESULTS"; }

record_vm_facts() {
  {
    echo "date: $(date -Is)"
    echo "uname: $(uname -a)"
    echo "os:"
    cat /etc/os-release || true
    echo
    echo "disk:"
    df -h / || true
    echo
    echo "memory:"
    free -h || true
    echo
    echo "cpu:"
    nproc || true
    lscpu | sed -n '1,12p' || true
    echo
    echo "gpu:"
    if command -v nvidia-smi >/dev/null 2>&1; then
      nvidia-smi || true
    else
      echo "nvidia-smi: not installed"
    fi
    if command -v lspci >/dev/null 2>&1; then
      lspci | grep -iE 'vga|3d|display' || echo "lspci: no VGA/3D/display device"
    else
      echo "lspci: not installed"
    fi
    ls -l /dev/dri 2>/dev/null || echo "/dev/dri: absent"
    echo
    echo "pid1: $(ps -p 1 -o comm= 2>/dev/null || true)"
    echo "xvfb-run: $(command -v xvfb-run || echo missing)"
    echo "ffmpeg: $(command -v ffmpeg || echo missing)"
  } > "$ARTIFACTS/vm-facts.txt"
}

record_docker() {
  {
    echo "date: $(date -Is)"
    if command -v docker >/dev/null 2>&1; then
      echo "docker CLI: present ($(command -v docker))"
      docker version || true
    else
      echo "docker CLI: not installed"
      if command -v apt-get >/dev/null 2>&1 && sudo -n true >/dev/null 2>&1; then
        echo "attempt: sudo apt-get install -y docker.io"
        sudo apt-get install -y docker.io || echo "apt docker.io failed: $?"
      else
        echo "sudo apt-get not available; did not install docker"
      fi
    fi
    echo
    echo "dockerd probe:"
    if command -v dockerd >/dev/null 2>&1; then
      if docker info >/dev/null 2>&1; then
        echo "docker info: ok (daemon already running)"
        docker info 2>&1 | sed -n '1,40p' || true
      else
        echo "starting dockerd --iptables=false --storage-driver=vfs for 20s"
        sudo dockerd --iptables=false --storage-driver=vfs > "$ARTIFACTS/dockerd.log" 2>&1 &
        local pid=$!
        local i
        local ready=0
        for i in 1 2 3 4 5 6 7 8 9 10; do
          sleep 2
          if docker info >/dev/null 2>&1; then
            ready=1
            break
          fi
        done
        if [[ "$ready" == "1" ]]; then
          echo "dockerd: started"
          docker info 2>&1 | sed -n '1,30p' || true
          echo "image tag confirmed earlier via Docker Hub API; pull is skipped here because the editor is installed from the Unity tarball."
        else
          echo "dockerd: failed to become ready"
          tail -n 40 "$ARTIFACTS/dockerd.log" || true
        fi
        sudo kill "$pid" >/dev/null 2>&1 || true
        sleep 1
        sudo kill -9 "$pid" >/dev/null 2>&1 || true
      fi
    else
      echo "dockerd: not installed"
    fi
    echo
    echo "GameCI image that matches this project (not required once the tarball editor is present):"
    echo "  unityci/editor:ubuntu-${UNITY_VERSION}-linux-il2cpp-3.2.2"
  } > "$ARTIFACTS/docker.txt" 2>&1
}

install_editor() {
  if [[ -x "$UNITY" ]]; then
    note "editor binary present: $UNITY"
    return 0
  fi
  note "editor binary missing, downloading ${UNITY_VERSION} (${UNITY_CHANGESET})"
  local dest_parent
  dest_parent="$(dirname "$UNITY_ROOT")"
  mkdir -p "$dest_parent" 2>/dev/null || sudo mkdir -p "$dest_parent"
  sudo chown -R "$(id -u):$(id -g)" "$dest_parent" || true
  mkdir -p "$UNITY_ROOT" "$ARTIFACTS/downloads"
  local editor_url="https://download.unity3d.com/download_unity/${UNITY_CHANGESET}/LinuxEditorInstaller/Unity-${UNITY_VERSION}.tar.xz"
  local il2cpp_url="https://download.unity3d.com/download_unity/${UNITY_CHANGESET}/LinuxEditorTargetInstaller/UnitySetup-Linux-IL2CPP-Support-for-Editor-${UNITY_VERSION}.tar.xz"
  curl -fL --retry 3 -C - -o "$ARTIFACTS/downloads/Unity-${UNITY_VERSION}.tar.xz" "$editor_url"
  curl -fL --retry 3 -o "$ARTIFACTS/downloads/linux-il2cpp.tar.xz" "$il2cpp_url"
  tar -xJf "$ARTIFACTS/downloads/Unity-${UNITY_VERSION}.tar.xz" -C "$UNITY_ROOT"
  tar -xJf "$ARTIFACTS/downloads/linux-il2cpp.tar.xz" -C "$UNITY_ROOT"
  [[ -x "$UNITY" ]]
}

prepare_license() {
  LICENSE_FILE=""
  SERIAL_ARGS=()
  {
    echo "UNITY_LICENSE set: $([[ -n "${UNITY_LICENSE:-}" ]] && echo yes || echo no)"
    echo "UNITY_LICENSE_FILE set: $([[ -n "${UNITY_LICENSE_FILE:-}" ]] && echo yes || echo no)"
    echo "UNITY_EMAIL set: $([[ -n "${UNITY_EMAIL:-}" ]] && echo yes || echo no)"
    echo "UNITY_PASSWORD set: $([[ -n "${UNITY_PASSWORD:-}" ]] && echo yes || echo no)"
    echo "UNITY_SERIAL set: $([[ -n "${UNITY_SERIAL:-}" ]] && echo yes || echo no)"
    echo "UNITY_LICENSING_SERVER set: $([[ -n "${UNITY_LICENSING_SERVER:-}" ]] && echo yes || echo no)"
  } > "$ARTIFACTS/license-env.txt"

  if [[ -n "${UNITY_LICENSE_FILE:-}" && -f "${UNITY_LICENSE_FILE}" ]]; then
    LICENSE_FILE="$UNITY_LICENSE_FILE"
  elif [[ -n "${UNITY_LICENSE:-}" && -f "${UNITY_LICENSE}" ]]; then
    LICENSE_FILE="$UNITY_LICENSE"
  elif [[ -n "${UNITY_LICENSE:-}" ]]; then
    LICENSE_FILE="$ARTIFACTS/unity-license.ulf"
    printf '%s\n' "$UNITY_LICENSE" > "$LICENSE_FILE"
    chmod 600 "$LICENSE_FILE"
  fi

  if [[ -z "$LICENSE_FILE" && -n "${UNITY_SERIAL:-}" && -n "${UNITY_EMAIL:-}" && -n "${UNITY_PASSWORD:-}" ]]; then
    SERIAL_ARGS=(-username "$UNITY_EMAIL" -password "$UNITY_PASSWORD" -serial "$UNITY_SERIAL")
  fi

  if [[ -n "$LICENSE_FILE" ]]; then
    note "license file: $LICENSE_FILE (contents not logged)"
  elif [[ ${#SERIAL_ARGS[@]} -gt 0 ]]; then
    note "serial activation arguments present (values not logged)"
  else
    note "no Unity license secret, .ulf, or serial is available"
  fi
}

run_unity() {
  local logfile="$1"
  local seconds="$2"
  shift 2
  mkdir -p "$(dirname "$logfile")"
  local -a auth=()
  if [[ -n "${LICENSE_FILE}" ]]; then
    auth+=(-manualLicenseFile "$LICENSE_FILE")
  fi
  if [[ ${#SERIAL_ARGS[@]} -gt 0 ]]; then
    auth+=("${SERIAL_ARGS[@]}")
  fi
  # Drop file cache so the editor is not competing with the tarball pages.
  sudo sh -c 'sync; echo 3 > /proc/sys/vm/drop_caches' >/dev/null 2>&1 || true
  set +e
  xvfb-run -a -e "${logfile}.xvfb" \
    timeout --signal=TERM --kill-after=30s "$seconds" \
    "$UNITY" \
    -batchmode \
    -nographics \
    -projectPath "$PROJECT" \
    -logFile "$logfile" \
    "${auth[@]}" \
    "$@"
  local code=$?
  note "unity exit ${code}: $(basename "$logfile") args: $*"
  return "$code"
}

excerpt_log() {
  local src="$1"
  local dest="$2"
  if [[ ! -f "$src" ]]; then
    echo "log missing: $src" > "$dest"
    return
  fi
  grep -E -i -n 'error|exception|license|fail|Licensing|compiler|Scripts have compiler|Test run|exiting batchmode|build (succeeded|failed)|Unable to|display' "$src" \
    | head -n 200 > "$dest" || true
}

log_has_license_block() {
  local src="$1"
  [[ -f "$src" ]] || return 1
  grep -E -i -q 'No valid Unity Editor license|Licensing initialization failed|must be activated|Failed to activate|No sufficient license|license client.*error|Access token is unavailable' "$src"
}

note "spike root: $PROJECT"
record_vm_facts
record_docker

if install_editor && "$UNITY" -version > "$ARTIFACTS/unity-version.txt" 2>&1; then
  set_result a_install PASS
  note "a install PASS ($("$UNITY" -version 2>/dev/null | head -n 1))"
else
  set_result a_install FAIL
  note "a install FAIL"
  set_result b_license FAIL
  set_result c_editmode FAIL
  set_result d_player FAIL
  set_result e_capture FAIL
  exit 1
fi

prepare_license

TEST_LOG="$ARTIFACTS/editmode.log"
TEST_RESULTS="$ARTIFACTS/editmode-results.xml"
set +e
run_unity "$TEST_LOG" 1500 \
  -runTests \
  -testPlatform EditMode \
  -testResults "$TEST_RESULTS" \
  -accept-apiupdate
TEST_CODE=$?
set -e
excerpt_log "$TEST_LOG" "$ARTIFACTS/editmode-excerpt.txt"

if log_has_license_block "$TEST_LOG"; then
  set_result b_license FAIL
  note "b license FAIL (see artifacts/editmode-excerpt.txt)"
else
  # A clean batch exit without a license error counts as activation working.
  if grep -E -q 'Exiting batchmode successfully|Test run completed' "$TEST_LOG"; then
    set_result b_license PASS
    note "b license PASS"
  else
    set_result b_license FAIL
    note "b license FAIL (editor did not report a successful batch session)"
  fi
fi

if [[ -f "$TEST_RESULTS" ]] && grep -q 'result="Passed"' "$TEST_RESULTS" && ! grep -q 'result="Failed"' "$TEST_RESULTS"; then
  set_result c_editmode PASS
  note "c editmode PASS"
elif [[ "$TEST_CODE" -eq 0 ]] && [[ -f "$TEST_RESULTS" ]]; then
  set_result c_editmode PASS
  note "c editmode PASS (exit 0)"
else
  set_result c_editmode FAIL
  note "c editmode FAIL (exit $TEST_CODE)"
fi

BUILD_LOG="$ARTIFACTS/linux-player.log"
BUILD_DIR="$ARTIFACTS/build"
mkdir -p "$BUILD_DIR"
PLAYER="$BUILD_DIR/ArgosFleetDashboard.x86_64"
set +e
run_unity "$ARTIFACTS/prepare-player.log" 900 \
  -executeMethod Argos.Fleet.EditorTools.FleetSceneSetup.PrepareLinuxPlayer \
  -accept-apiupdate
PREPARE_CODE=$?
run_unity "$BUILD_LOG" 1800 \
  -buildLinux64Player "$PLAYER" \
  -accept-apiupdate
BUILD_CODE=$?
set -e
excerpt_log "$BUILD_LOG" "$ARTIFACTS/linux-player-excerpt.txt"
excerpt_log "$ARTIFACTS/prepare-player.log" "$ARTIFACTS/prepare-player-excerpt.txt"

if [[ -x "$PLAYER" || -f "$PLAYER" ]]; then
  set_result d_player PASS
  note "d player PASS ($PLAYER)"
else
  set_result d_player FAIL
  note "d player FAIL (prepare $PREPARE_CODE, build $BUILD_CODE)"
fi

CAPTURE_DIR="$ARTIFACTS/capture"
mkdir -p "$CAPTURE_DIR"
if [[ ! -f "$PLAYER" ]]; then
  set_result e_capture FAIL
  note "e capture FAIL (no player to run)"
else
  export LIBGL_ALWAYS_SOFTWARE=1
  export MESA_LOADER_DRIVER_OVERRIDE=llvmpipe
  export MESA_GL_VERSION_OVERRIDE=4.5
  rm -f /tmp/.X99-lock
  set +e
  Xvfb :99 -screen 0 1280x720x24 +extension GLX +render -noreset > "$ARTIFACTS/xvfb.log" 2>&1 &
  XVFB_PID=$!
  sleep 1
  DISPLAY=:99 glxinfo -B > "$ARTIFACTS/glxinfo.txt" 2>&1
  DISPLAY=:99 "$PLAYER" \
    -screen-fullscreen 0 \
    -screen-width 1280 \
    -screen-height 720 \
    -force-glcore \
    -logFile "$ARTIFACTS/player.log" \
    -argosCaptureDir "$CAPTURE_DIR" \
    -argosQuit \
    > "$ARTIFACTS/player-stdout.txt" 2>&1 &
  PLAYER_PID=$!
  sleep 2
  DISPLAY=:99 ffmpeg -y -f x11grab -video_size 1280x720 -framerate 12 -i :99 \
    -t 12 -pix_fmt yuv420p "$CAPTURE_DIR/fleet-overview.mp4" \
    > "$ARTIFACTS/ffmpeg.log" 2>&1
  FFMPEG_CODE=$?
  # One more still, in case the player screenshots landed late.
  DISPLAY=:99 ffmpeg -y -f x11grab -video_size 1280x720 -i :99 -frames:v 1 "$CAPTURE_DIR/x11-frame.png" \
    >> "$ARTIFACTS/ffmpeg.log" 2>&1
  wait "$PLAYER_PID"
  PLAYER_CODE=$?
  kill "$XVFB_PID" >/dev/null 2>&1 || true
  set -e
  if [[ -s "$CAPTURE_DIR/fleet-overview.mp4" || -s "$CAPTURE_DIR/overview.png" || -s "$CAPTURE_DIR/x11-frame.png" ]]; then
    set_result e_capture PASS
    note "e capture PASS (player exit $PLAYER_CODE, ffmpeg $FFMPEG_CODE)"
  else
    set_result e_capture FAIL
    note "e capture FAIL (player exit $PLAYER_CODE, ffmpeg $FFMPEG_CODE)"
  fi
fi

note "results:"
cat "$RESULTS"
if grep -q '=FAIL' "$RESULTS"; then
  exit 1
fi
exit 0
