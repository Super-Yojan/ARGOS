# Native Google 3D Tiles Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement task by task.

**Goal:** Render and navigate Google Photorealistic 3D Tiles in this GPUI app.

**Architecture:** A native Rust worker selects and decodes tiles, renders them
with wgpu, and delivers completed BGRA frames to GPUI. Downloads run in a bounded
pool, independently of the UI and renderer. Camera and hierarchy math use f64.

**Tech Stack:** Rust, GPUI CE 0.2.2, wgpu 27, glam, gltf, draco-core,
reqwest, image, serde, url, bytemuck, pollster, crossbeam-channel.

**Spec:** ../specs/2026-10-19-native-google-3d-tiles-design.md

## Global Constraints

- Native Rust; target macOS first.
- Credentials come from GOOGLE_MAPS_API_KEY; do not log authenticated URLs.
- Bound resident memory, pending jobs, viewport size, and decoded allocations.
- Keep attribution synchronized to displayed content.
- Preserve existing untracked files; no worktree is possible from an unborn HEAD.

## Review Focus

- Tile URLs must preserve session parameters without leaking credentials.
- Distant Earth coordinates must stay precise after conversion to GPU floats.
- Replacement refinement must retain parent coverage until children are ready.
- Draco attribute IDs and mappings must match glTF attributes.
- Resizes and stale worker results must not regress the displayed frame.

### Task 1: Camera and authenticated hierarchy

**Files:** maps/camera.rs, maps/tiles.rs, maps/tests.rs; Cargo dependencies.
**Interfaces:** Camera produces view/projection and visibility/SSE tests; TileTree
owns transformed nodes and selects render IDs plus prioritized download IDs.

- [x] Add failing tests for WGS84, URL propagation/origin restrictions,
  inherited transforms, and replacement coverage.
- [x] Implement double-precision camera math and conservative bounds.
- [x] Implement explicit tilesets, authenticated requests and bounded scheduling.
- [x] Verify the core tests pass, including local HTTP response handling.

### Task 2: Tile content

**Files:** maps/content.rs, content fixtures/tests.
**Interfaces:** decode(bytes, source, fetch) -> TileContent with primitives,
local transforms, decoded RGBA textures, and copyright sources.

- [x] Add failing textured GLB and Draco mapping fixture tests.
- [x] Decode glTF triangles, materials and images; explicitly decode Draco.
- [x] Validate sizes, required extensions and malformed inputs.
- [x] Verify fixtures and attribution tests pass.

### Task 3: Native GPU renderer

**Files:** maps/renderer.rs, maps/shader.wgsl.
**Interfaces:** Renderer uploads TileContent, renders selected tile IDs using
Camera, and returns bounded BGRA bytes plus dimensions.

- [x] Add a GPU smoke test with occluding textured triangles and odd row width.
- [x] Implement textured pipelines, depth testing, origin-relative uniforms,
  offscreen rendering and padded asynchronous readback.
- [x] Run the smoke test and inspect its rendered artifact.

### Task 4: GPUI lifecycle and integration

**Files:** maps.rs, maps/worker.rs, main.rs, MAPS.md.
**Interfaces:** MapView::new(MapConfig, &mut Context<Self>); MapConfig defaults
to a San Francisco neighborhood and reads GOOGLE_MAPS_API_KEY at startup.

- [x] Add lifecycle/viewport tests for pixel budgets and stale generation IDs.
- [x] Connect bounded loader pool, cache, session refresh, and renderer worker.
- [x] Add camera gestures, keyboard navigation/reset, loading/error and credits.
- [x] Launch the app; exercise missing key and offline smoke mode.
- [x] Run formatting, full tests and clippy, then inspect the whole change.
- [x] Attempt live Google verification if a key becomes available; otherwise
  explicitly report that limitation and provide launch instructions.

## Execution ledger

Ruling: Work in the supplied directory because the repository has no commits,
all project files are untracked, and the user asked to build this implementation.
Do not create an unrelated initial commit or alter the git index.

Ruling: Execute inline in this session following the user's "Build it" instruction.
Dependency manifest changes outside the writable src root require sandbox approval.


## Final validation ledger

- Task 1: complete. Camera/hierarchy regression tests pass; local HTTP fixture
  verifies nested JSON loading, key/session propagation, and redacted failures.
- Task 2: complete. Textured GLB, Khronos Draco, generated RGB Draco with explicit
  attribute mappings, malformed ranges, unknown required extensions, and f64
  node translations pass.
- Task 3: complete. Native GPU smoke checks textured occlusion, BGRA channels,
  padded row removal, image orientation, Survey mode, and scene clearing.
- Task 4: implementation complete. GPUI opens the offline fixture; build and
  Clippy pass. Native CUA inspection failed because the native pipe could not
  start, so interactive window/input verification remains manual.
- Full suite: `cargo test -- --include-ignored` (using the temporary target
  directory and native GPU/loopback permissions): 18 passed, 0 failed.
- `cargo clippy --all-targets -- -D warnings`: passed. `cargo build`: passed.
- Live Google verification subsequently passed after the user authorized GCP key
  retrieval. HTTP 200 root response; actual San Francisco geometry rendered at
  960 × 600 with data attribution. `.env` is mode 0600 and Git-ignored.
- Fresh code review findings (RGB Draco, f64 node transforms, empty selections)
  were corrected and have regression coverage.
- Ruling: the user's supplied city-map reference replaces the initial monochrome
  chrome. PRODUCT.md and DESIGN.md record the dark blue-green HUD; maps/hud.rs
  implements it. Survey mode tones data in the shader; Imagery restores colors.
- Ruling: leave changes in the user's supplied checkout without staging or
  committing unrelated untracked files in this unborn repository.

- Final UI review findings corrected: Tab/Shift-Tab traversal and control tab
  stops, pointer occlusion for the HUD, relevant-error recomputation, and an
  always-available reload action. Added mailbox tests for stale generations and
  preservation of pending GPU frames when status messages arrive.
- Live compatibility: recognize 3DTILES_content_gltf and check its declared
  required glTF extensions. Corrected identity/TRS bounding-sphere scale to avoid
  inflated culling volumes and unnecessary refinement.

- Final full suite with the configured key: 24 passed, 0 failed, 0 ignored.
  Live check rendered 95 tiles at 960 × 600 with attribution in 20 seconds.
  Final build and Clippy with warnings denied both passed.

## Follow-up: latency and 2D / 3D

User requested lag diagnosis and a mode switch. Added north-up orthographic 2D
with pointer-scaled panning, preserved 3D orbit, compass-adjacent buttons and 2/3
shortcuts. Added pipeline wall-clock metrics; these exclude GPUI GPU compositor
timing. Corrected frame acceptance: camera motion presents completed frames in
order, while resize, mode, imagery and reset changes invalidate prior epochs.
A fresh reviewer found viewport-dependent 2D drag scale; corrected with a
regression for 400 px and 1080 px window heights.

Final validation: 30 passed, including GPU and live Google 3D → 2D → 3D checks.
Clippy with warnings denied, build and formatting pass. In the same scripted
120-update motion test, accepted frames improved from 69 to 100–103 across
repeat runs; cached render/readback medians were 3–4 ms. Network/decode samples
were separate and substantially longer. No claim of directly profiling the
GPUI compositor was made.
