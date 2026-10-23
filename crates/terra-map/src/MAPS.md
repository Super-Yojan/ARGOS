# terra-map

The application renders Google Photorealistic 3D Tiles in Rust using wgpu,
with GPUI providing the window, controls, and frame presentation.

The local Cargo package is `terra-map`; import it as `terra_map`. In the ARGOS
workspace it lives at `crates/terra-map`. It exports `Map`, `MapConfig`,
`MapView`, `MapMode`, `GeoPosition`, `ObjectId`, `Color`, `ObjectStatus`,
`MapEvent`, and `MapError`. The `learn-gpui` binary remains the standalone
explorer. The ARGOS dashboard binary is `argos-gpui`, which mounts `MapView`.
The package has not been published. No API key is committed.

## Use from your application

Add a local dependency in the consuming application's Cargo.toml:

```toml
[dependencies]
terra-map = { path = "../terra-map" }
gpui-kit = "=0.8.0"
```

The map view uses the GPUI types re-exported by `gpui-kit` 0.8.0. Use
`gpui_kit::{prelude::*, *}` in your UI modules and call `gpui_kit::init(cx)`
before `gpui_kit::open_window(...)`. Its builder returns an `Entity`; apply
`.expect(...)` to the result of `open_window`, outside the builder closure.
Do not mix these types with a separate `gpui-ce` dependency: the frameworks
provide distinct `Context`, `Window`, `RenderImage`, and `IntoElement` types.

Create the controller outside GPUI, then mount its view inside a GPUI context:

```rust
use terra_map::{Map, MapConfig, MapView, GeoPosition, Color, MapMode};

let map = Map::new(MapConfig::default())?;
let robot = map.add_3d_obj(
    GeoPosition::new(37.7749, -122.4194, 80.0),
    "assets/robot.glb",
)?;
map.illuminate_3d_object(robot, Color::rgb(255, 160, 50))?;

// Inside your GPUI window/view construction:
let map_view = cx.new(|cx| MapView::new(map.clone(), cx));
```

Your fleet owns the listener and robot types. Capture a controller clone in its
selection callback; no GPUI context is required to send map commands:

```rust
let on_robot_clicked = {
    let map = map.clone();
    move |latitude: f64, longitude: f64| {
        map.show_lat_lon(latitude, longitude)
    }
};
// In your listener: let location = robot.get_location();
on_robot_clicked(37.7760, -122.4180)?;
```

`show_lat_lon` recenters once with a smooth 500 ms transition. It preserves zoom,
target altitude, heading/pitch and 2D/3D mode. Later selections replace the current
transition; dragging, scrolling, keyboard navigation and reset cancel it. Set
`MapConfig.fly_duration` to `Duration::ZERO` for an immediate jump. Use
`show_position(GeoPosition)` to also change the target's altitude. Commands sent
before mounting are retained; a clone shares the same map, rather than creating
another map. Use one `MapView` for each controller.

```rust
map.set_object_position(robot, GeoPosition::new(37.775, -122.419, 80.0))?;
map.set_object_heading(robot, 90.0)?;
map.set_object_scale(robot, 2.0)?;
map.clear_illumination(robot)?;
map.set_mode(MapMode::TwoD);
map.set_mode(MapMode::ThreeD);
// When the robot leaves the fleet:
map.remove_object(robot)?;
```

Adding a model schedules work; success means the command was accepted. Once a
view has a viewport, check `map.object_status(robot)?` for `Loading`, `Ready`, or
`Failed(message)`, or drain `map.poll_event()` for `ObjectLoaded` and
`ObjectLoadFailed`. The event queue retains the most recent 256 notifications;
status is authoritative. Removing an instance invalidates its handle and discards
its queued notifications. Handles from another map are rejected.

## Fleet example

```sh
# Three selectable robots, alert toggles, and host-owned 2D/3D controls:
cargo run -p terra-map --example fleet
# Use your own static local model:
cargo run -p terra-map --example fleet -- /absolute/path/to/robot.glb
# No API key or network needed:
MAPS_DEMO=1 cargo run -p terra-map --example fleet
```

The example defines its own `Robot` and selection handler in
`src/examples/fleet.rs`. It includes a generated robot model, displayed at 12×
scale and an absolute 80 m ellipsoid height for neighborhood-scale visibility.
Set `MapConfig.show_controls = false` to embed the viewport in a host layout;
Google Maps attribution remains visible.

## Model conventions and limits

- Supported local files: static `.glb` and `.gltf`, including Draco. Wavefront
  `.obj`, animation playback, skeletal deformation, and terrain clamping are not
  implemented. Meshes use the glTF default/base pose.
- Positions are WGS84 latitude/longitude degrees and absolute ellipsoid height in
  meters, **not height above the ground**. Supply terrain height plus vehicle
  offset when you want ground placement. glTF units are meters; Y-up is converted
  to a local east/north/up frame. Heading rotates clockwise about local up; scale
  is uniform and positive.
- Highlighting adds an unlit color tint per instance. It does not cast light onto
  the surroundings or add a bloom effect. Clearing it restores the model material.
- Relative model paths resolve from the process working directory. External glTF
  resources must stay within the model's directory tree. File resolution and
  decoding run on a background thread. Canonical paths, including symlink aliases,
  share decoded geometry and GPU buffers/textures; instance uniforms are separate.
- Default limits: 4,096 objects and a 128 MiB aggregate decoded model cache. A
  single outstanding decode reserves remaining capacity; buffers, decoded image
  dimensions, and geometry are checked against it. GPU resources, instance
  uniforms, encoded input (at most 32 MiB per model including external files),
  parser data, and decoder scratch are additional memory. This is a resource
  budget, not a total process memory cap. Unused assets are released. Failures
  remain failed until the instance is removed and added again. Initial Google
  connection failures leave local objects usable; use Retry to reconnect tiles.
  A terminal renderer/worker error marks existing objects failed and rejects new
  additions with `MapError::WorkerUnavailable` until the worker is restarted.

## Launch the explorer

From the repository root, launch with a key available in the process environment:

```sh
export GOOGLE_MAPS_API_KEY='your-key'
cargo run -p terra-map
```

For the local `.env` file, launch from the repository root with:

```sh
set -a
source .env
set +a
cargo run -p terra-map
```

`.env` is ignored by Git; `.env.example` contains the supported settings.

The Google Cloud project must have billing and **Map Tiles API** enabled.
Key restrictions must permit requests from this native application. A key for
another Google API alone does not enable Map Tiles API access. No Cesium ion
account or token is needed. The application does not read `.env` files.

The interface follows the supplied dark blue-green city-map reference. Survey
mode applies a subdued cyan tone in the GPU shader; Imagery restores source
colors. The HUD shows live camera and renderer state.

The initial view is San Francisco. Set the initial coordinates, height,
heading, pitch, distance, screen-space error and memory/frame budgets using
`MapConfig` when creating your controller.

- Use the **2D / 3D** buttons next to the compass, or press **2 / 3**.
- 2D is a north-up orthographic view of the same Google tiles. Drag to pan;
  switching back restores the prior 3D heading and pitch.
- In 3D, drag to orbit; Shift-drag to pan. Scroll zooms in both modes.
- Arrow keys or WASD move; plus/minus zoom; R or Home resets the view.
- Click the map to focus keyboard controls.

Run the synthetic textured fixture without an API key or Google requests:

```sh
MAPS_DEMO=1 cargo run -p terra-map
```

The fixture is explicitly labeled offline and contains no Google map data.

## Validation

```sh
cargo test -p terra-map
cargo test -p terra-map gpu_ -- --ignored
# Optional: uses the live Google API with the configured key.
cargo test -p terra-map live_google_tiles_render -- --ignored --nocapture
cargo clippy -p terra-map --all-targets -- -D warnings
```

The second command needs native GPU access. It checks model sharing, isolated
highlights, async removal, missing-key fallback, depth, texture color,
orientation and padded readback at 257 × 129 pixels, and writes
`/private/tmp/learn-gpui-map-smoke.png` on macOS.

Fixtures include a generated textured quad and the Khronos glTF sample Box
with Draco compression. The Box fixture is from
https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Box
and is licensed CC BY 4.0; see `maps/fixtures/README.md`.

## Implementation boundaries

The supported path is explicit 3D Tiles hierarchies containing textured glTF
triangle meshes, including Draco geometry. Unsupported required extensions,
implicit tiling, multiple tile contents, point clouds and legacy b3dm/i3dm
containers report errors. This is a native Google tiles viewer, not the complete
Cesium rendering engine.

Downloads and decoding use four background workers. Resident tile resources
have a configurable 384 MiB budget. Tile payloads stay in memory and are not
written to disk. Visible fallback tiles remain until their replacements load.
Session URLs are refreshed before the documented three-hour minimum expires.
Google Maps attribution and visible tile copyright sources are shown below
the viewport.

wgpu renders geometry on the GPU. Completed BGRA frames are copied through
CPU memory into GPUI images, so performance depends on viewport size and
hardware. Frame resolution defaults to at most 1,440,000 pixels. This initial
implementation does not use platform-specific zero-copy texture sharing.

Dropping the view signals its workers to exit. An already running HTTP request
may finish or hit its timeout before its loader exits; shutdown never joins a
network worker on the UI thread.

Google API access and dataset rendering require a live check with your key;
offline fixtures only verify the rendering/decoding path.


## Diagnosing loading and motion

The Rendering panel now separates render/readback, GPU wait/copy, latest tile
request, decode/upload, pending requests, resident tiles, and UI polling/image
preparation. These are CPU wall-clock stage timings. GPU wait includes driver
scheduling, rendering, transfer and synchronization; it is not a GPU timestamp.
UI preparation excludes GPUI's compositor/GPU texture upload.

Tile requests and decoding run in the loader pool, so high download times
explain delayed detail without proving slow UI drawing. Camera movement can
render already resident tiles while refinement continues. Completed frames are
presented in generation order instead of being discarded whenever another mouse
movement arrives. Resize, 2D/3D changes, imagery changes and reset invalidate old
frame epochs, preserving visual consistency across those transitions.

The renderer still copies completed frames through CPU memory into GPUI. This
has overhead and is a potential future optimization; the added measurements do
not establish that GPUI's compositor itself is a bottleneck.
