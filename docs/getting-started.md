# Getting started

The operator dashboard is the native GPUI binary `argos-gpui`. It mounts the Map View from `crates/terra-map`. Install Git, stable Rust with your platform's C/C++ linker, and the system libraries GPUI needs to open a window (on Linux: Wayland or X11, `libxkbcommon`, and Vulkan headers).

## Open the map

From the repository root:

```sh
MAPS_DEMO=1 cargo run -p argos-gpui
```

`MAPS_DEMO=1` draws the crate's synthetic fixture and does not call Google. To load Photorealistic 3D Tiles, export `GOOGLE_MAPS_API_KEY` in the same process and omit `MAPS_DEMO`. The repository does not contain a key. The Google Cloud project needs billing and the Map Tiles API.

The window is the map plus the rover list. Empty-space drag orbits in 3D, Shift-drag pans, and scroll zooms. Dragging a selected rover marker sends a goal on release and does not pan. Outdoor GPS uses that same drag on the map marker: release sends the WGS84 goal, and there is no separate confirm. The camera opens at `ARGOS_MAP_START_LAT` / `ARGOS_MAP_START_LON` (default 38.8297, -77.3075) and recenters on the fleet once rover positions arrive. It does not jump on every later pose. `GOOGLE_MAPS_API_KEY` can live in `.env`; a real environment variable overrides the file. A missing key leaves the window open and names that variable in the sidebar. `MAPS_DEMO=1` does not need a key.

`cargo run -p argos-gpui` uses the hosted fleet (`tls/argos.34.135.204.210.sslip.io:7448`, prefix `terra/phone`, user `terra`). Copy `.env.example` to `.env` and set `ARGOS_ZENOH_PASSWORD` there. The password has no default and is not in git. Without it the window still opens and the sidebar names that variable. `MAPS_DEMO=1` keeps working. Local Zorvane is the commented block in `.env.example` (`tcp/127.0.0.1:7447`, prefix `terra/rover`, no user). The window publishes a goal, an autonomy level, or teleop only for the selected rover. Model loading is described in `crates/terra-map/src/MAPS.md`. The repository README lists every variable and the local-metre origin.

## Service

The Rust service is separate from the window. A loopback simulated fleet:

```sh
cargo run -p argos-server -- --demo --dev
```

`--dev` is restricted to loopback simulated fleets. A live deployment requires GitHub OAuth, an HTTPS origin, and explicit viewer/operator numeric account IDs. The service no longer serves a React bundle. `ARGOS_WEB_DIR` (default `web/dist`) is only used when you place static files there yourself.

Docker builds the same service:

```sh
docker compose up -d --build
```

Check `http://127.0.0.1:8080/healthz`. That URL is the API, not a dashboard. Real fleets use `compose.fleet.yaml` and the [hosting notes](browser-dashboard.md#connect-a-real-fleet-from-local-docker).

Start Zorvane using its [current run guide](https://super-yojan.dev/Zorvane/running/). For physical hardware, install a compatible TerraPhone runtime and verify its reported hardware and authority state. Connection never arms hardware automatically. The GPUI window arms only after Take control and a one-second hold, and it waits for the rover's hardware status before showing armed.

## Checks

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p terra-map
```

GPU tests stay ignored unless you pass `--ignored` on a machine with a GPU. Transport tests bind loopback sockets. Set `ARGOS_CHECK_TYPES=1` to compare `crates/argos-server/tests/generated.ts` with the Rust contract instead of rewriting it.

## Documentation site

```sh
python3 -m pip install -r requirements-docs.txt
mkdocs serve
mkdocs build --strict
```

The site is published at [super-yojan.dev/ARGOS](https://super-yojan.dev/ARGOS/).
