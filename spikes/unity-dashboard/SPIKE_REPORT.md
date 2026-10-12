# Unity dashboard spike report

Feasibility only. A cloud agent tried to build a rough ARGOS 3D fleet view in Unity 6 LTS and run the headless loop on this VM. Mock telemetry only. No Zenoh. No edits to ARGOS Rust or Swift.

The design source is `docs/design/3d-dashboard.md` on `main` (merged from PR #23). The prototype is `spikes/unity-dashboard/`: Unity `6000.3.26f1` (`de8167e7440c`), 20 placeholder rovers from `Assets/StreamingAssets/fleet-telemetry.json`, cyan / amber / red status color, solid completed paths, dashed planned paths, ghosted when the pose is older than 2.5 s, and a selection that dims the rest. EditMode tests for that stale rule live in `Assets/Tests/EditMode/TelemetryParserTests.cs`. `run-spike.sh` is the local a–e loop. `.github/workflows/unity-dashboard-spike.yml` is the optional GameCI loop (`workflow_dispatch` only).

## Pass / fail

| Step | Result | Evidence |
| --- | --- | --- |
| a. Install the Unity editor | **PASS** | Host binary `/opt/unity/6000.3.26f1/Editor/Unity -version` prints `6000.3.26f1`. GameCI image `unityci/editor:ubuntu-6000.3.26f1-linux-il2cpp-3.2.2` also runs that binary and prints `6000.3.26f1`. |
| b. License activation | **FAIL** | No license secret, `.ulf`, or serial in the environment. Editor exits **198**. |
| c. EditMode tests, NUnit XML | **FAIL** | Same exit 198. The test runner never started. `artifacts/editmode-results.xml` was not written. |
| d. Linux player | **FAIL** | `-buildLinux64Player` exits 198 before any compile or IL2CPP link. No player binary. |
| e. Xvfb screenshots and ~10 s video | **FAIL** | Not run. There was no player to launch. No screenshot and no video. |

`b` is the blocker for `c`–`e` on this VM. The commands for `c` and `d` were still invoked. They die in the license client, before script compilation.

## Mac (owner machine)

The owner ran the same project on an Apple Silicon Mac with Unity `6000.6.5f1` and a valid Personal license. The project file still pins `6000.3.26f1`; the newer editor opened it.

| Step | Result |
| --- | --- |
| License | **PASS** — Personal license accepted in `-batchmode` |
| EditMode tests | **PASS** — 9/9 |
| Mac player | **PASS** — `-buildOSXUniversalPlayer`, 67 MB, Mono |
| Capture | **PASS** — `-argosCaptureDir` and `-argosQuit` wrote `overview.png` and `selected.png` |

Two compile/build fixes were required before that run, and they are in this branch now:

1. `Assets/Editor/FleetSceneSetup.cs` needs `using UnityEditor.Build;`. Without it, `NamedBuildTarget` is unresolved (`CS0103` at the `SetScriptingBackend` call).
2. `ProjectSettings/ProjectSettings.asset` had `scriptingBackend` Standalone set to `1` (IL2CPP). The Mac has no IL2CPP module, so the player build failed with `Currently selected scripting backend (IL2CPP) is not installed`. The committed default is Mono (`0`). `PrepareLinuxPlayer` still sets IL2CPP immediately before the Linux player build.

The overview frame shows about 10 of the 20 rovers. The other two rows are inside the old frustum on paper, but they do not appear in the capture: the fleet sits in the lower half of the 1280×720 image, and the ground used the same ZWrite-off transparent shader as the rovers, so the ground quad could paint over the far markers. The scene camera is now at `(0, 36, -12)`, pitched 72°, field of view 40, which places all four rows between roughly 27% and 73% of the frame height. The ground uses an opaque unlit shader so it draws before the rovers. That framing change has not been re-captured; the images below are the owner's build from before it.

![Mac overview, before the framing change](docs/mac-overview.png)

![Mac selection, terra-07 ring, other rovers dimmed](docs/mac-selected.png)

## Exact errors

Host editor (`spikes/unity-dashboard/artifacts/editmode.log`), and the same text from the GameCI `unity-editor -quit` entrypoint:

```
[Licensing::Module] Error: Access token is unavailable; failed to update
[Licensing::Client] Error: Code 404 while processing request (status: Found 0 entitlement groups and 0 free entitlements matching requested entitlement ids)
[Licensing::Module] Error: 'com.unity.editor.headless' was not found.
No valid Unity Editor license found. Please activate your license.
```

Process exit code: **198**. `xvfb-run` itself only logged harmless `xkbcomp` keysym warnings.

Checked env names, all unset: `UNITY_LICENSE`, `UNITY_LICENSE_FILE`, `UNITY_EMAIL`, `UNITY_PASSWORD`, `UNITY_SERIAL`, `UNITY_LICENSING_SERVER`. `gh secret list` on this repo returned HTTP 403, so GitHub secret names could not be listed from this run. Nothing in the process environment supplied a license. No Unity account was created and no one else's credentials were used.

## VM facts

| Fact | Value |
| --- | --- |
| OS | Ubuntu 24.04.4 LTS, kernel `6.12.94+`, x86_64 KVM guest |
| CPU | 4 vCPU, Intel Xeon (family 6 model 207) |
| RAM | 15 GiB, no swap. About 4.8 GiB available when the editor started. The license abort happened before a heavy import, so IL2CPP memory use was not measured. |
| Disk | 254 GiB volume. After the editor install, about 20 GiB used and 234 GiB free. |
| GPU | No `nvidia-smi`. `lspci` is not installed. `/dev/dri` is absent. No hardware GL device. |
| Display tools | `xvfb-run` and `ffmpeg` are installed. They were not asked to capture a window. |
| Init | PID 1 is `tini`, not systemd. `invoke-rc.d` refused to start `docker.service` (`policy-rc.d` denied, no sysv script). |
| Editor install | Official tarball `Unity-6000.3.26f1.tar.xz` (4,540,380,840 bytes) plus `LinuxEditorTargetInstaller/UnitySetup-Linux-IL2CPP-Support-for-Editor-6000.3.26f1.tar.xz` (59,392,156 bytes). The old `TargetSupportInstaller/` URL for that module returns 404. Installed under `/opt/unity/6000.3.26f1` (about 8.4 GiB). `ldd` on `Editor/Unity` reported no missing libraries after the apt packages in `run-spike.sh`'s notes (GTK, NSS, Mesa, clang 18, lld). |
| Linux player module | The IL2CPP support archive contains both IL2CPP and Mono player variations. The project sets Standalone scripting backend to IL2CPP (`scriptingBackend: Standalone: 1`). The build never got that far. |
| Docker | Not installed at boot. `sudo apt-get install -y docker.io` installed Docker 29.1.3. Manual `sudo dockerd --iptables=false --storage-driver=vfs` reaches `sudo docker info` (cgroup v2, vfs). The first `run-spike.sh` probe recorded "failed to become ready" because it called `docker info` without sudo; the socket is root-owned. That probe is fixed to use sudo. |
| GameCI image | `docker pull unityci/editor:ubuntu-6000.3.26f1-linux-il2cpp-3.2.2` succeeded. `docker images` reported the image at 10.4 GiB. vfs storage grew `/var/lib/docker` to about 194 GiB while unpacking it (disk 85% full, 40 GiB free). The image was deleted after the version and license checks. Disk returned to about 20 GiB used. Do not leave this image on the vfs driver. |

## What the owner must provide

A Personal license cannot be activated headlessly on this VM. Unity's licensing client has no account token, so the headless entitlement `com.unity.editor.headless` is missing. Activation has to happen once in Unity Hub, on a machine the owner controls (the Mac is the natural place). This spike did not verify that a `.ulf` created on macOS is accepted here. GameCI documents that file as portable across OS and editor version. If Unity rejects it as machine-bound, the working alternative is a paid serial or a floating license server.

### Personal (free)

1. Install Unity Hub and sign in with the Unity account that should own CI.
2. Unity Hub → Preferences → Licenses → Add → **Get a free personal license**. Hub can show a license before it writes the file. Use Add so the `.ulf` is created.
3. Copy that file. GameCI's paths:
   - macOS: `/Library/Application Support/Unity/Unity_lic.ulf` (also check `~/Library/Application Support/Unity/Unity_lic.ulf`)
   - Linux: `~/.local/share/unity3d/Unity/Unity_lic.ulf`
   - Windows: `C:\ProgramData\Unity\Unity_lic.ulf`
4. GitHub → this repo → Settings → Secrets and variables → Actions. Create:
   - `UNITY_LICENSE` — the `.ulf` XML pasted as-is, not base64
   - `UNITY_EMAIL` — the Unity account email
   - `UNITY_PASSWORD` — the Unity account password

Leave `UNITY_SERIAL` unset for a Personal license. GameCI says not to mix the Personal file with a Professional serial.

Local rerun on a machine that already has the editor:

```bash
UNITY_LICENSE_FILE=/path/to/Unity_lic.ulf ./spikes/unity-dashboard/run-spike.sh
```

`UNITY_LICENSE` may be either a path to the `.ulf` or the raw XML. The script writes a temp file and passes `-manualLicenseFile`. It does not print the file.

### Professional serial

Subscribe, copy the serial (`XX-XXXX-XXXX-XXXX-XXXX-XXXX`), and set:

- `UNITY_SERIAL`
- `UNITY_EMAIL`
- `UNITY_PASSWORD`

Do not also set `UNITY_LICENSE`. The script and the workflow pass the serial through to the editor as `-serial` / `-username` / `-password` (the workflow via the GameCI actions' own env handling).

### Floating license server

Point GameCI at the server with the action input `unityLicensingServer`. The local script reads `UNITY_LICENSING_SERVER` only to record whether it is set; it does not yet pass it through. A server URL was not available here.

## How CI consumes it

`.github/workflows/unity-dashboard-spike.yml` is `workflow_dispatch` only, so it does not fail every pull request while the secrets are absent. After the secrets exist, run it by hand. It uses:

- `game-ci/unity-test-runner@v4` — EditMode, project `spikes/unity-dashboard`, Unity `6000.3.26f1`, NUnit results uploaded as `editmode-results`
- `game-ci/unity-builder@v4` — `targetPlatform: StandaloneLinux64` (the `ubuntu-6000.3.26f1-linux-il2cpp-3.2.2` image)
- a follow-up step that launches the player with `-force-glcore` under Xvfb and records about 12 seconds with `ffmpeg`

Both jobs pass `UNITY_LICENSE`, `UNITY_EMAIL`, `UNITY_PASSWORD`, and `UNITY_SERIAL` from Actions secrets. GameCI writes the `.ulf` and adds `-manualLicenseFile` for the Personal path. Those actions return the license after the step when the activation method needs it.

GitHub-hosted `ubuntu-24.04` runners have the same shape of problem this VM has for step e: no GPU. `-force-glcore` plus Mesa llvmpipe is the attempt, and it was not reached here, so that capture step is still unproven.

## Recommendation

Do the Unity dashboard work on the owner's Mac. That machine already proved the loop: a Personal license, 9/9 EditMode tests, a 67 MB Mono player, and the two captures. This VM can install Unity 6.3 LTS, and the GameCI editor image runs here, but it cannot compile, test, build, or screenshot the project until a license file exists. After that file is a secret, EditMode tests are a reasonable cloud/CI job. The 3D loop is a poor fit for this VM even then: there is no GPU, llvmpipe was never exercised, RAM is 15 GiB with no swap, and the GameCI image on Docker's vfs driver occupied about 194 GiB for a 10.4 GiB image. Scene iteration, frame timing at 20 rovers, and the look of the status language belong on the Mac, where Unity Hub, a GPU, and the rest of ARGOS already are.
