# Browser dashboard

ARGOS’s primary interface is a React/TypeScript dashboard backed by the existing Rust fleet client. The browser supports monitoring, explicit arm/disarm, waypoint send/cancel, held manual driving, and emergency stop. The native apps remain a legacy fallback for safety reset, target-search setup, proposal approval, and other mission workflows.

## Develop on Windows, Linux or macOS

Install Git, stable Rust (including the platform's C/C++ linker), and Node.js 22.12 or newer. Windows Rust requires the Visual Studio C++ build tools; Linux needs a C compiler and standard development tools. No Xcode, phone, Tailscale account or rover is needed for the simulated fleet.

From the repository root:

```sh
npm ci --prefix apps/web
npm run dev
```

Open `http://127.0.0.1:5173`, then **Open local demo**. The command starts the Rust service on loopback port 8080 and Vite on loopback port 5173. First startup compiles Rust dependencies. The demo includes an armed rover, a disarmed rover, and a rover with unknown pose, including a 64-bit ID. Take control, select rover 1, enter local coordinates, review and confirm a waypoint. Arrival is simulated over five seconds.

The local view uses an offline SVG metre grid. The geographic view shows raw
GPS fixes directly, with accuracy and fix age. No compass, AR tracking, shared
origin, or eight-second alignment is required for GPS display. Fixes older than
three seconds disappear; coarse fixes remain visible with their reported
uncertainty. Missing positions remain unknown, and local coordinates are never
substituted for GPS.

The GPS map uses a grayscale OpenStreetMap street basemap by default, with visible
attribution and normal browser caching. It requests only the active viewport;
there is no offline download or tile prefetch feature. An HTTPS MapLibre style
can override it through `ARGOS_TILE_STYLE`. Usage follows the
[OpenStreetMap tile policy](https://operations.osmfoundation.org/policies/tiles/).
A failed basemap does not hide the GPS coordinates in rover details.

GPS display is observation only. Waypoint drafting, occupancy overlays and
movement currently use the local view and the selected rover's local frame.
Clicking the GPS map never drafts or sends a driving command. Outdoor GPS
navigation needs a separate vehicle-side navigation capability; a raw fix does
not authorize it. Live Terra frames are not automatically aligned to one another.
Rovers without a shared local frame remain selectable outside the local field.

Run the processes separately if preferred:

```sh
cargo run -p argos-server -- --demo --dev
npm --prefix apps/web run dev
```

Development identity is explicitly enabled by `--dev`, only with the simulated fleet, loopback bind address and loopback origin. It is rejected in production. Local sessions and command audit records are saved in the ignored `argos.db` file. The service applies schema version 1 automatically on first start.

## Vehicle dimensions and avoidance tuning

Select a rover, take control, and open **Dimensions & avoidance**. Quick ± buttons and numeric inputs edit length, width, obstacle-clearance margin, planner linear/turn speed limits, and prediction look-ahead. **Apply settings** sends one validated update; reported configuration is updated only after Terra acknowledges the exact values and next configuration revision. **Reset draft** restores reported values.

Length/width are planning geometry, not hardware measurements. The planner currently uses a conservative circular footprint: `radius = hypot(length, width) / 2 + clearance`. Initial 0.85 m dimensions and approximately 0.049 m clearance preserve the pre-existing 0.65 m radius. Enter the real vehicle's dimensions. These limits tune the obstacle-avoidance planner; L2 direct waypoint still bypasses avoidance as labeled.

Changes require fresh, ready, disarmed/non-arming hardware, no active goal or pending command, current run/authority/configuration revisions, and an operator claim. Terra validates again at receipt and invalidates cached routes/trajectories after application. It never arms or drives as a result of configuration. Limits: length/width 0.1–3 m; clearance 0–1 m; planner speed 0.05–2 m/s; planner turn rate 0.1–2 rad/s; look-ahead 0.5–5 s. Unknown cells and braking safeguards stay enabled.

This requires the updated Terra iOS framework/app. Older phone builds show an update-required message rather than pretending to apply settings. Configuration is session-scoped: retained through controller reset and recording transitions, but defaults return when Terra restarts. Install using the usual sibling Terra build workflow.

## Occupancy maps

Select a rover to display its observed occupancy grid under the fleet markers. **Occupancy map** toggles the layer in the local view. White cells are observed free space; black cells (occupancy ≥65) are occupied; gray cells are unknown. All rover markers remain visible. **Fit fleet** includes the selected grid bounds. Occupancy remains in its observed local frame and is not overlaid on raw GPS.

Map age is displayed independently of pose age. Grids older than 2.5 seconds are faded and explicitly marked stale. Missing data shows **No occupancy grid received from Terra**; the dashboard never substitutes a synthetic map for a real vehicle. Terra must be actively publishing mapping data. Old grids are invalidated on localization-frame changes and hidden when their run does not match current authority.

The browser polls the selected rover's authenticated `/api/v1/rovers/{id}/occupancy` endpoint once per second. Grids are validated and limited to 250,000 cells; display supports dimensions up to 4096 per side. Rover IDs and sequences remain strings to preserve 64-bit precision. Large cell arrays are kept out of the 5 Hz fleet event stream.

## Autonomy levels

Select a rover and take control to request a supported level. The browser exposes: L0 fully teleop, L1 assisted teleop, L2 waypoint without obstacle avoidance, L3 obstacle-aware waypoint, L4 target search, and unavailable L5 decision making. Unsupported choices are disabled based on Terra's `supported_levels` report. The selector requests authority only; the separate Manual driving panel enables held input. Target-search mission setup remains in the native interfaces.

Mode changes require fresh pose and authority and are bound to the reviewed run/revision. They do not arm the hardware. Previous unpublished dashboard intent is retired; a matching vehicle acknowledgement confirms selection. L2 explicitly has no obstacle avoidance. L3 requires a fresh obstacle map, while L4 requires supported hardware and live person detection. A mode may be acknowledged while its safety readiness remains on hold; the panel shows Terra's reported reason, such as `map_stale`, separately from pose freshness.

## Arm or disarm a vehicle

Select its marker on the fleet field, choose **Take control**, then **Arm vehicle → Confirm arm rover …** in Vehicle power. Arming requires live pose, ready disarmed hardware and clear safety authority, each less than 0.5 seconds old. The server binds the request to the reviewed run and authority revision; Terra still validates it and arms the hardware locally. The dashboard reports success only after matching hardware-token telemetry confirms the armed state.

**Disarm vehicle** requires operator control but can be sent with stale telemetry. It retires pending dashboard intent. Hardware requests are audited and never automatically retried; missing confirmation after ten seconds is shown as uncertain. No arming occurs when connecting, selecting a rover or taking control.

## Manual driving and emergency stop

Select a rover, choose **Take control**, then **Take over manual driving**. If necessary, the dashboard requests Teleop and waits for the matching vehicle acknowledgement. Hardware must already be explicitly armed. Hold WASD, arrow keys, or a touch direction to move; release to stop. The browser requests 0.3 m/s forward/reverse and 0.6 rad/s turning, within the server's 0.5 m/s and 1 rad/s bounds. Keep the vehicle in view.

Moving input requires a healthy link, online membership, pose/hardware/authority younger than 0.5 seconds, matching run/revision, and armed ready manual authority. Input is ordered per browser owner; the service assigns monotonic wire sequences in its own session. Requests queued for 250 ms are rejected. Held input expires after 450 ms without a valid pulse, checked by the fleet worker's 200 ms loop. These are software gates, not a real-time braking guarantee.

Release, window blur, page hiding, rover changes, claim loss, and stale authority clear driving intent. Reconnecting never resumes held input. Network loss can prevent a neutral packet arriving, so Terra's onboard deadman and hardware watchdog remain required. A failed or delayed request disables browser driving until explicit takeover again.

**Emergency stop** attempts the vehicle's latched safety stop, goal cancellation, and hardware disarm, then releases the tab's claim. It remains available to the controlling operator with stale telemetry. The result distinguishes publication from acknowledgement; inspect reported vehicle state and physical motion. Stop audit records capture the operator and publication outcome. Audit-storage failure is reported after the stop paths are attempted and does not prevent stopping.

Safety reset remains a separate action in the legacy native operator app. Inspect the vehicle, reset safety there, then explicitly rearm and take over again. The browser never clears a stop latch by selecting a mode. The demo's stop latch lasts until the demo service restarts.

Rust tests cover gate/lease/order behavior and real local Zenoh drive/neutral/stop/disarm messages. Browser integration covers held keys, release, and demo emergency disarm. Physical latency, braking distance, and phone lifecycle still require vehicle trials.

## Verify a change

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
npm test --prefix apps/web
npm run build --prefix apps/web
npx --prefix apps/web playwright install chromium
npm run test:e2e --prefix apps/web
```

Playwright starts both development processes automatically. Stop a manually started development server first for an isolated run. Integration tests open loopback sockets. Linux CI runs the existing Rust checks and browser tests; a separate job runs service tests, frontend tests and builds on all three desktop operating systems.

API types are Rust-owned. After changing a contract, run `cargo test -p argos-server --test types` and commit `apps/web/src/generated.ts`. Set `ARGOS_CHECK_TYPES=1` to check drift without rewriting the generated file; CI does this automatically.

## Local Docker deployment

Install Docker Desktop on macOS or Windows, or Docker Engine with the Compose
plugin on Linux. Start Docker, then run from the repository root:

```sh
docker compose up -d --build
```

Open `http://127.0.0.1:8080` and choose **Open local demo**. Use this exact origin
for the demo's same-origin command checks. No `.env` file, OAuth app, rover, Rust,
or Node.js installation is needed. Port 8080 must be free. Source changes require
running the build command again; source files are not live-mounted.

The root `compose.yaml` builds one image containing the frontend and Rust
service, plus a small Caddy proxy container. Both share a container network
namespace: the Rust server keeps its required loopback bind, while Caddy forwards
the port published on the host's `127.0.0.1`. This uses Docker bridge networking
on Linux and Docker Desktop. Development identity is restricted to simulated
vehicles; real fleets use the authenticated hosting configuration below.

```sh
docker compose logs -f
docker compose down
```

Sessions and audit history persist in the `argos-local-data` volume across
restarts. `docker compose down -v` deletes that local demo data. Simulated vehicle
state resets whenever the service restarts. Check service health at
`http://127.0.0.1:8080/healthz` and fleet readiness at `/readyz`.

## Connect a real fleet from local Docker

The separate `compose.fleet.yaml` connects the browser dashboard to a real Zenoh
fleet. The browser runs at `https://localhost:8443`, with GitHub sign-in and
numeric user-ID allowlists. The Rust API is reachable only inside the Compose
network; only the HTTPS proxy is published, on host loopback. Demo login remains
disabled. This configuration can run alongside the demo and keeps separate data.

1. Copy the configuration template from the repository root:

   ```sh
   cp .env.fleet.example .env.fleet
   ```

2. Register a **separate GitHub OAuth app** for this local dashboard. Set homepage
   URL to `https://localhost:8443` and callback URL to
   `https://localhost:8443/auth/callback`. Put its client ID and secret in
   `.env.fleet`; set `ARGOS_OPERATORS` to your numeric GitHub user ID, not your
   username. Optional `ARGOS_VIEWERS` grants observation without command access.
   These settings are separate from the public dashboard's OAuth app.

3. Set the fleet endpoint, prefix and membership credentials to match the Terra
   phones. The template starts with the hosted TLS router documented in
   [the GCP guide](../packaging/gcp/README.md), prefix `terra/phone` and user
   `terra`; enter its fleet password privately in `.env.fleet`. Router availability
   and valid credentials are still required. Keep the filled-in file out of git.
   For passwords containing `$` or `#`, use single quotes around the entire value
   so Compose reads it literally.

   For a LAN router, set e.g. `ARGOS_ZENOH_ENDPOINT=tcp/192.168.1.10:7448`.
   For a router on the Docker host, use `tcp/host.docker.internal:7448` (or port
   7447 and prefix `terra/rover` for Zorvane). The Compose file supplies the host
   mapping for Linux as well as Docker Desktop. The router must accept connections
   from Docker's network; on Linux a listener bound only to `127.0.0.1` is not
   reachable through the bridge gateway. Bind to a reachable private interface
   and allow the Docker subnet through the firewall. Use unauthenticated TCP only
   on a trusted private network. For a router without authentication, delete both
   `ARGOS_ZENOH_USER` and `ARGOS_ZENOH_PASSWORD` lines from `.env.fleet`.

4. Start the dashboard:

   ```sh
   docker compose --env-file .env.fleet -f compose.fleet.yaml up -d --build
   ```

5. Caddy issues a local HTTPS certificate. Export its root certificate:

   ```sh
   docker compose --env-file .env.fleet -f compose.fleet.yaml cp proxy:/data/caddy/pki/authorities/local/root.crt ./argos-local-root.crt
   ```

   Trust this certificate in your browser or operating system before signing in.
   On macOS, import it into Keychain Access and set **Always Trust** for SSL.
   On Windows, import it into **Trusted Root Certification Authorities** for your
   user. On Linux, use the distribution's CA import mechanism; Firefox may require
   a separate import in its certificate settings. Only export `root.crt`; keep the
   private CA key inside the Docker volume. See
   [Caddy's local HTTPS documentation](https://caddyserver.com/docs/automatic-https#local-https).

6. Open **https://localhost:8443** and sign in with the allowed GitHub account.
   Start Terra publishing to the same router and prefix. Its rovers appear as
   telemetry arrives; connecting does not arm them. Use the existing explicit
   control, arm and waypoint actions when ready.

Inspect connectivity with:

```sh
docker compose --env-file .env.fleet -f compose.fleet.yaml logs -f argos proxy
curl --cacert argos-local-root.crt https://localhost:8443/healthz
curl --cacert argos-local-root.crt https://localhost:8443/readyz
```

`/healthz` checks HTTP availability. `/readyz` requires a healthy fleet and may
return 503 until live telemetry arrives. A disconnected fleet produces no demo
replacement. Check endpoint reachability, TLS hostname, membership credentials,
and matching topic prefix if no vehicles appear.

Stop with `docker compose --env-file .env.fleet -f compose.fleet.yaml down`.
Volumes preserve sessions, audit history and the local certificate authority.
Adding `-v` deletes all three, requiring you to import the new root certificate
after the next startup. Source or configuration changes require rerunning the
startup command. Use the public hosting setup below to serve other computers.

## Public hosting for one team

The browser talks only to the ARGOS HTTPS service. The service reaches the existing fleet router through the Linux host's Tailscale network. Browser users do not need Tailscale. Zenoh TCP is never exposed publicly by this deployment.

1. Provision a Linux VM with Docker Compose and Tailscale. Point a domain's DNS at it; permit public TCP 80/443. Restrict fleet-router access through tailnet policy to the ARGOS host and required Zenoh port.
2. Register a GitHub OAuth app with callback `https://YOUR_DOMAIN/auth/callback`. No repository permissions are requested. Obtain stable numeric GitHub account IDs through GitHub's user API.
3. Copy `packaging/web/.env.example` to `packaging/web/.env`, set the domain, origin, OAuth credentials, explicit viewer/operator account lists and fleet endpoint/prefix. Keep this file private and outside git.
4. Start with `ARGOS_FLEET=demo`; leave `ARGOS_DEV_AUTH` disabled. From the repository root run:

```sh
docker compose -f packaging/web/compose.yaml up -d --build
```

5. Sign in through GitHub and verify authorized/unauthorized accounts, viewer restrictions, control ownership, draft confirmation, cancellation and reconnect behavior against simulated data. Configure and verify a geographic basemap if needed.
6. Set `ARGOS_FLEET=zenoh`, ensure the router is reachable privately, and recreate the service with the same Compose command. Validate against Zorvane before operating real hardware.

Caddy supplies HTTPS; Axum serves the frontend and API from one origin. Production is one service instance with one fleet worker and SQLite volume. Do not scale to multiple replicas: command claims are in memory. The production Compose configuration targets Linux hosts; the local Docker setup above and native development commands are cross-platform.

GitHub tokens are used server-side during sign-in and are not persisted. Session cookies are HttpOnly, Secure in hosted mode, SameSite=Lax, and expire after eight hours. Numeric-ID allowlists determine roles on every request. Mutations require same-origin and CSRF validation. OAuth state is single-use and bound to a browser cookie; PKCE protects the code exchange.

`/healthz` reports the HTTP process; `/readyz` returns 200 only while the fleet link is healthy. Structured logs report service, request and fleet connection state. SQLite's `audit` table records operator IDs, request IDs, rover IDs, publication and acknowledgment transitions. Browser history shows the most recent 100 operations.

## Command semantics

- One browser tab holds a 30-second renewable command claim; it renews every ten seconds. Viewers cannot acquire it or issue commands. Closing the event stream releases its claim; expiry also discards unpublished work.
- The service preserves the 90-second high-level supervision horizon and 150-second acknowledgment timeout. Ages are measured since service receipt, not synchronized end-to-end rover time.
- Drafts and requests are bound to the observed rover run and localization frame, including local-only frames. Before publishing, the service checks membership, pose, armed/ready hardware, authority, coordinate bounds and supported mode. It waits for the matching mode token, then rechecks readiness. L2 `waypoint_direct` is preserved if already selected; otherwise it requests L3 `waypoint`.
- Published means published, not accepted. The rover's matching goal token confirms acceptance. Cancel requires a newly received idle report after the cancellation request. No command is retried automatically.
- Request UUIDs and operation transitions are persisted before publication. A repeated request ID returns its original operation; changed payloads conflict. After restart, unfinished operations become interrupted/uncertain and are never resumed.
- Disconnect clears browser drafts and unpublished service work. A goal already published to Terra continues. Hardware arming is explicit in the browser Vehicle power panel.
- Browser claims cannot fence native clients. Do not issue native commands concurrently with a browser operator. Browser emergency stop publishes the vehicle safety latch, cancels the goal, and requests disarm. Retain independent vehicle stop/watchdog protections; a network publication is not proof of physical braking.

## Persistence, backup and rollback

The `argos-data` volume contains `argos.db` and its SQLite WAL. Use SQLite's online backup command rather than copying a live database file alone:

```sh
docker compose -f packaging/web/compose.yaml exec argos sqlite3 /data/argos.db '.backup /data/argos-backup.db'
```

Copy the backup out of the volume and store it securely off-host; sessions and audit records are sensitive. Schedule daily backups in the host's existing backup system and test a restore on a demo deployment. Expired sessions and OAuth state are safe to remove; keep audit records according to the team's retention policy.

Before upgrading, take a backup. To roll back, rebuild/deploy the previous code revision with a compatible schema; version 1 is additive. For a restore, stop the service, replace the database from backup and remove only its obsolete WAL/SHM files, restore ownership to UID 10001, then restart. Restart recovery discards unfinished command intent, so confirm rover-reported status before issuing new commands.

The geographic renderer and worker are bundled locally; no CDN script is needed.

No hosted deployment or real-hardware command is performed by installing or building these files. A domain, VM, GitHub credentials and fleet connectivity must be supplied for actual hosting.
