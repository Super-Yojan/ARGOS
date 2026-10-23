# Native Google Photorealistic 3D Tiles in GPUI

Status: approved and implemented; live Google tiles rendered and inspected.

## Outcome and agreed scope

Implement `src/maps.rs` so this GPUI application can display and navigate
Google Photorealistic 3D Tiles using native Rust. The user chose native Rust
over a webview, has a Google API key, and approved the scope of streaming,
textured rendering, camera controls, level of detail, and attribution.

Target macOS first, matching the current development environment. Keep the
renderer and tile loader portable Rust. Read credentials at runtime from
`GOOGLE_MAPS_API_KEY`. No matching environment variable or project environment
file was visible during inspection; live verification requires configuring
the key in the process that launches the application.

Success means real Google tiles appear in the existing GPUI window, remain
geographically aligned while navigating, refine as the camera approaches,
and show attribution for the currently displayed content. A synthetic mesh
alone does not satisfy completion.

## Existing application

`maps.rs` is empty. `main.rs` opens a counter using `gpui-ce = 0.2.2` and
`gpui_ce_platform = 0.1.0`. No map loader, camera, renderer, or credential
handling exists. The project files are currently untracked and the repository
has no commits. Preserve unrelated working files and do not include them in
documentation or implementation commits.

## Approach and alternatives

Use a Rust tile client, glTF decoder, and an offscreen `wgpu` renderer.
This matches the requested language and allows the map to be a GPUI view.

An alternative is Cesium Native through a C++ bridge. It offers mature tile
selection but still requires renderer integration and introduces C++ builds
and FFI. Reserve it for a future requirement for broad Cesium compatibility.
CesiumJS in a webview was considered and the user selected native Rust.

This implementation targets Google's explicit tile hierarchies and textured
triangle meshes. It is not a general implementation of every 3D Tiles format.
Unsupported required extensions must produce an actionable error rather than
silently displaying incomplete geometry.

## Module boundaries

- `maps.rs`: public `MapView` and `MapConfig`, GPUI lifecycle, interaction,
  loading state, attribution, and error display.
- `maps/camera.rs`: WGS84/ECEF conversions, local coordinate frame, projection,
  camera movement, bounding volumes, frustum tests, and screen-space error.
- `maps/tiles.rs`: tileset parsing, inherited transforms/refinement, URL
  resolution, request scheduling, session management, and resident tile cache.
- `maps/content.rs`: GLB/glTF mesh and image decoding, Draco decompression,
  node transforms, materials, and content copyright extraction.
- `maps/renderer.rs` and `maps/shader.wgsl`: GPU resources, textured depth-tested
  rendering, offscreen targets, and frame transfer to GPUI.
- `main.rs`: instantiate the map instead of the example counter.
- `Cargo.toml` and `Cargo.lock`: add the dependencies required by these modules.

Use `wgpu`, `glam`, `gltf`, `draco-core`, `image`, `reqwest`, `url`,
`serde`/`serde_json`, and small supporting crates as required. Select compatible
versions during implementation and verify Draco support with a fixture before
connecting the renderer. Do not assume parsing glTF alone decompresses Draco.

## Data flow and threading

The UI sends the latest camera and viewport state to a renderer worker.
A bounded download/decode pool supplies decoded tiles. Network requests,
decompression, GPU submission, and readback must not block GPUI's UI thread.
Queue sizes and resident memory are bounded; obsolete camera updates are
coalesced. A frame carries a generation number so old work cannot overwrite
a newer frame or resized viewport. Dropping the view stops its workers.

Start from Google's root tileset endpoint:
`https://tile.googleapis.com/v1/3dtiles/root.json`.
Resolve content URLs relative to their containing tileset and preserve the
session and other inherited query parameters. Attach the configured API key
to Google tile requests. Restrict credential propagation to the Google tile
origin; credentials must not follow cross-origin redirects or content links.
Redact query strings from user-facing errors and logs.

Load nested external tilesets as hierarchy nodes. Compose parent and child
tile transforms in double precision. Support bounding boxes, spheres, and
geographic regions conservatively, including regions crossing the date line.
Preserve inherited `ADD` or `REPLACE` refinement semantics.

Select visible tiles by frustum and screen-space geometric error. Keep parent
content visible until the replacement children needed for the view are ready.
Apply bounded concurrent requests, deduplication, request priority, and an
in-memory LRU cache that does not evict visible fallback content. Obey response
cache directives; do not persist Google tile payloads to disk. Refresh the root
session before its documented lifetime ends, and invalidate session-specific
pending requests without discarding a usable displayed frame prematurely.

## Content decoding and coordinates

Decode indexed and non-indexed triangle primitives, node transforms,
positions, UVs, vertex colors, base-color factors/textures, and sampler state.
Handle embedded and same-origin external buffers/images. Support
`KHR_draco_mesh_compression` with attribute IDs from the glTF extension;
honor attribute point mappings when rebuilding vertices. Validate byte ranges,
accessors, image dimensions, and decode allocation limits before allocation.

Honor glTF Y-up to tile Z-up conversion exactly once. Apply transforms in
double precision, then subtract the local render origin before converting to
GPU floats. Keep geographic coordinates in double precision to avoid jitter.
Use textured unlit color for photogrammetry, with correct sRGB conversion,
depth testing, winding, alpha modes, and double-sided material behavior.

## Native GPU presentation

Render into an offscreen color texture with a depth attachment. Initially
transfer completed frames through asynchronous GPU readback into GPUI
`RenderImage`. GPUI 0.2.2 documents BGRA image data; explicitly convert channels
and strip GPU row padding before presenting. Never retain an unbounded series
of old images or GPU buffers.

This path renders geometry on the GPU but adds GPU-to-CPU-to-GPU frame transfer.
It avoids relying on an unsupported shared GPU device. GPUI's macOS surface API
accepts CoreVideo pixel buffers, so direct texture sharing would require a
separate platform-specific integration and is deferred. Render on camera,
viewport, or tile-content changes and keep at most one readback in flight.
Cap internal render resolution to a configurable pixel budget for large or
Retina windows. Smoothness must be measured; do not promise a frame rate.

## View behavior and configuration

Open over San Francisco at a neighborhood-scale view, with configurable
initial latitude, longitude, altitude, heading, and pitch in `MapConfig`.
Expose drag-to-orbit, modified drag-to-pan, wheel-to-zoom, and reset-view
controls. Clamp pitch and distance to avoid singularities and accidental
movement through the target. Update the viewport from actual layout bounds
and device scale; handle minimized or zero-sized windows without rendering.

Keep controls visually minimal and follow the application's black-and-white
styling. Show loading progress, a useful missing-key message, and recoverable
network/authentication errors. Retain the last usable scene when an individual
tile fails. Retry transient failures with bounded backoff and respect
`Retry-After`; do not repeatedly retry permission failures.

Display Google Maps brand attribution plus the copyright sources collected
from the glTF assets actually rendered. Split semicolon-separated sources,
aggregate their occurrences, and order by frequency with deterministic ties.
Update attribution with the displayed frame, including fallback parent tiles.

## Validation and acceptance

1. Unit fixtures cover URL resolution and key/session propagation without
   leakage, inherited tile transforms/refinement, WGS84 conversions, frustum
   culling, screen-space error, and attribution aggregation.
2. Content fixtures cover a textured GLB and a Draco-compressed mesh, including
   non-identity attribute mappings. Malformed content fails safely.
3. A local HTTP fixture verifies nested tilesets, bounded loading, replacement
   fallback, and handling of authorization/transient failures without Google
   credentials or API charges.
4. A GPU smoke render verifies textured geometry, depth, image orientation,
   channel order, resize behavior, and padded readback rows.
5. Run formatting, compilation, relevant tests, and lint checks. Use a build
   output directory in a permitted location when required by the sandbox.
6. With `GOOGLE_MAPS_API_KEY` configured and Map Tiles API enabled, perform a
   live application check: real tiles visible, navigation/refinement working,
   viewport resizing stable, attribution visible, and memory bounded while
   moving between neighborhoods. Report any live check that cannot run;
   offline tests cannot establish Google API access or real dataset rendering.

## References

- Google Photorealistic 3D Tiles:
  https://developers.google.com/maps/documentation/tile/3d-tiles
- Google custom renderer requirements:
  https://developers.google.com/maps/documentation/tile/create-renderer
- Google attribution and cache policies:
  https://developers.google.com/maps/documentation/tile/policies
- OGC 3D Tiles specification source:
  https://github.com/CesiumGS/3d-tiles/tree/main/specification
- Cesium Native renderer integration (alternative approach):
  https://cesium.com/learn/cesium-native/ref-doc/rendering-3d-tiles.html
- GPUI 0.2.2 image and surface integration was checked against the installed
  `gpui-ce` source.


## Theme amendment supplied during implementation

The user supplied a six-panel city visualization reference and requested that
look for the theme. It replaces the original monochrome chrome with a dark
blue-green full-viewport map and instrument HUD, cyan accents, a heading compass,
real camera/rendering metrics, and Survey/Imagery controls. PRODUCT.md and
DESIGN.md record the durable rules. The shader provides the subdued Survey tone;
Imagery mode preserves source colors.


## Live verification

The user authorized retrieval of an existing GCP key and writing it to `.env`.
A Maps-compatible key from the active project was saved with mode 0600; `.env`
is Git-ignored. The root endpoint returned HTTP 200. A native worker then loaded,
decoded and GPU-rendered actual San Francisco tiles at 960 × 600 with attribution.
The initial live check exposed Google's `3DTILES_content_gltf` declaration;
explicit support and a regression test were added. The refined GPU frame was
visually inspected and contained city geometry. Native window automation remains
unavailable because its local pipe could not start.
