# Getting started

The operator dashboard is the native GPUI binary `argos-gpui`. It mounts the Map View from `crates/terra-map`. Install Git, stable Rust with your platform's C/C++ linker, and the system libraries GPUI needs to open a window (on Linux: Wayland or X11, `libxkbcommon`, and Vulkan headers).

## Open the map

From the repository root:

```sh
MAPS_DEMO=1 cargo run -p argos-gpui
```

`MAPS_DEMO=1` draws the crate's synthetic fixture and does not call Google. To load Photorealistic 3D Tiles, export `GOOGLE_MAPS_API_KEY` in the same process and omit `MAPS_DEMO`. The repository does not contain a key. The Google Cloud project needs billing and the Map Tiles API.

The window is the map, the console bar, and the selected rover's inspector. Empty-space drag orbits in 3D, Shift-drag pans, and scroll zooms. Dragging a selected rover marker sends a goal on release and does not pan. Outdoor GPS uses that same drag on the map marker: release sends the WGS84 goal, and there is no separate confirm. The camera opens at `ARGOS_MAP_START_LAT` / `ARGOS_MAP_START_LON` (default 38.8297, -77.3075) and recenters on the fleet once rover positions arrive. It does not jump on every later pose. `GOOGLE_MAPS_API_KEY` can live in `.env`; a real environment variable overrides the file. A missing key leaves the window open and names that variable in a map notice. `MAPS_DEMO=1` does not need a key.

`cargo run -p argos-gpui` uses the hosted fleet (`tls/argos.34.135.204.210.sslip.io:7448`, prefix `terra/phone`, user `terra`). Copy `.env.example` to `.env` and set `ARGOS_ZENOH_PASSWORD` there. The password has no default and is not in git. Without it the window still opens and a map notice names that variable. `MAPS_DEMO=1` keeps working. Local Zorvane is the commented block in `.env.example` (`tcp/127.0.0.1:7447`, prefix `terra/rover`, no user). The window publishes a goal, an autonomy level, or teleop only for the selected rover. Model loading is described in `crates/terra-map/src/MAPS.md`. The repository README lists every variable and the local-metre origin.

## Router

The app uses Zenoh directly. For the existing hosted TLS endpoint and migration
instructions, see the [GCP router guide](https://github.com/Super-Yojan/ARGOS/blob/main/packaging/gcp/README.md).
There is no HTTP application service or OAuth setup.

`docker compose up -d --build` starts an optional local router on
`tcp/127.0.0.1:7448`, without authentication. Configure both simulator and dashboard
to use the same endpoint and prefix. `docker compose down` stops it.

Start Zorvane using its [current run guide](https://super-yojan.dev/Zorvane/running/). For physical hardware, install a compatible TerraPhone runtime and verify its reported hardware and authority state. Connection never arms hardware automatically. The GPUI window arms only after Take control and a one-second hold, and it waits for the rover's hardware status before showing armed.

## Checks

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p terra-map
```

GPU tests stay ignored unless you pass `--ignored` on a machine with a GPU. Transport tests bind loopback sockets. Tests live in the repository-level `tests/` directory, grouped by crate.

## Documentation site

```sh
python3 -m pip install -r requirements-docs.txt
mkdocs serve
mkdocs build --strict
```

The site is published at [super-yojan.dev/ARGOS](https://super-yojan.dev/ARGOS/).
