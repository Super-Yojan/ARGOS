# Getting started

The primary dashboard runs in a browser on Windows, Linux, or macOS. Install Git, stable Rust with your platform's C/C++ linker, and Node.js 22.12 or newer.

## Local simulated fleet

```sh
npm ci --prefix apps/web
npm run dev
```

Open `http://127.0.0.1:5173` and choose **Open local demo**. This starts Vite on port 5173 and the Rust service on port 8080. Select a rover marker, choose **Take control**, and review a waypoint before confirming it. The demo models arrival over five seconds; it is not physical-vehicle evidence.

The local metre map works without external tiles. Geographic maps need a configured HTTPS tile style and a usable rover geographic reference. The dashboard does not invent an anchor for unknown frames.

## Connect a real fleet or Zorvane

Use the [authenticated hosting setup](browser-dashboard.md#public-hosting-for-one-team). Set `ARGOS_FLEET=zenoh`, `ARGOS_ZENOH_ENDPOINT` to the reachable router, and `ARGOS_ZENOH_PREFIX` to the vehicle publisher's prefix. The service defaults to `tcp/127.0.0.1:7448` and `terra/rover`; a standalone Zorvane listener commonly uses port 7447. Set the endpoint explicitly to match your topology.

`--dev` is intentionally restricted to loopback simulated fleets. A live deployment requires GitHub OAuth, an HTTPS origin, and explicit viewer/operator numeric account IDs. Browser users need HTTPS access to ARGOS, not direct Zenoh access.

Start Zorvane using its [current run guide](https://super-yojan.dev/Zorvane/running/). For physical hardware, install a compatible TerraPhone runtime and verify its reported hardware and authority state. Connection and taking control never arm hardware automatically.

## Checks

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
npm test --prefix apps/web
npm run build --prefix apps/web
npx --prefix apps/web playwright install chromium
npm run test:e2e --prefix apps/web
```

Browser tests launch their own development services. Stop an existing demo first. Transport tests bind loopback sockets; they require an environment that permits networking.

## Legacy Apple fallback

The retained `apps/apple` app requires macOS/Xcode and Apple Silicon Rust targets. Run `scripts/build-apple.sh`, then open `apps/apple/ARGOS.xcodeproj`. See the [legacy control guide](autonomy/README.md). Apple screenshots and recorded native trials are separate evidence from browser tests. New dashboard features belong in `apps/web`.

## Documentation site

```sh
python3 -m pip install -r requirements-docs.txt
mkdocs serve
mkdocs build --strict
```

The site is published at [super-yojan.dev/ARGOS](https://super-yojan.dev/ARGOS/).
