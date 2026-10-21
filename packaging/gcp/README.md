# GCP Zenoh router

ARGOS GPUI, Terra and Zorvane connect directly to this authenticated Zenoh router.
The HTTP application server has been removed. There are no OAuth logins, HTTP
command endpoints or server-side operator roles in the router deployment. An
independent, scoped observability collector stores diagnostic history and Rerun exports.

## Existing host

- Project: `glid-app`; VM: `argos-host`; zone: `us-central1-a`.
- Static address: `34.135.204.210` (`argos-public-ip`).
- Public fleet endpoint: `tls/argos.34.135.204.210.sslip.io:7448`.
- Topic prefix: `terra/phone`; fleet account: `terra`.
- Internal listener: `tcp/127.0.0.1:7449`; never expose this port publicly.
- Public ports: 80/443 for Caddy certificate issuance/renewal; 7448 for Zenoh TLS.

Keep the static IP reserved and those firewall rules intact. Caddy still obtains
and renews the public certificate for the hostname. `argos-router-tls.timer`
runs every five minutes, copies changed certificates into `/var/lib/argos/tls`
and restarts the router when needed. The router reads its key and certificate
there as the `argos` service user. Do not remove Caddy or its certificate storage.
The HTTPS `/healthz` response proves only that Caddy is available, not that the
router is connected or that a rover is publishing telemetry.

## Connect the desktop app

Copy `.env.example` to `.env` in the repository root. Use:

```sh
ARGOS_ZENOH_ENDPOINT=tls/argos.34.135.204.210.sslip.io:7448
ARGOS_ZENOH_PREFIX=terra/phone
ARGOS_ZENOH_USER=terra
ARGOS_ZENOH_PASSWORD=<fleet-membership-password>
```

Then run `cargo run -p argos-gpui -- --real`. Enter the same endpoint, prefix and
credentials in TerraPhone's fleet settings. Give simultaneous vehicles distinct
numeric rover IDs. For Zorvane use its `TERRA_ZENOH_CONNECT`,
`TERRA_ZENOH_PREFIX`, `TERRA_ZENOH_USER` and `TERRA_ZENOH_PASSWORD` variables.

An authorized administrator can obtain the password from root-only
`/etc/argos/fleet.password`. Do not commit credentials. Fleet membership permits
publishing/subscribing to fleet topics; it does not provide per-operator roles.
Desktop control claims are local UI state, not an exclusive router-wide lock.
Coordinate operators; vehicle runtime checks remain the final command boundary.

## Build and deploy

Build the router for the Debian x86_64 host (requires cargo-zigbuild and Zig):

```sh
rustup target add x86_64-unknown-linux-gnu
cargo zigbuild --release --locked --target x86_64-unknown-linux-gnu \
  -p argos-zenoh --bin argos-router
```

Prepare `/tmp/argos-release/bin/argos-router` from that build and copy the contents
of `packaging/gcp/` into `/tmp/argos-release/gcp/`. Transfer the bundle to the VM.
The VM needs Caddy, Python 3 and systemd installed. No web bundle or OAuth
credentials are required. Run on the VM:

```sh
sudo bash /tmp/argos-release/gcp/install.sh
sudo systemctl status caddy argos-router argos-router-tls.timer
sudo journalctl -u argos-router -u argos-router-tls --since '10 minutes ago'
```

The installer creates the service user/directories when needed, preserves existing
fleet credentials and custom user dictionary entries, installs the router, and
replaces Caddy's old reverse proxy with a status response. It disables/stops the
legacy `argos-server.service` when present and removes its installed unit file.
It leaves old server binaries, `/etc/argos/server.env`, SQLite data and web files
untouched for recovery; none are used by the router-only deployment.

The first certificate may not be available yet. The installer attempts TLS
initialization immediately and the timer retries after a failure. After a pending
certificate is issued, run `sudo systemctl start argos-router-tls.service` and
confirm the router is active. An unchanged certificate does not trigger a renewal
restart; installing a new router release does restart the active router to load it.
Expect a brief disconnect during deployment or certificate rotation.

## Verify and operate

```sh
gcloud compute ssh argos-host --project=glid-app --zone=us-central1-a
sudo systemctl status caddy argos-router argos-router-tls.timer
sudo ss -ltnp
sudo journalctl -u argos-router --since '10 minutes ago'
```

From a client, `nc -vz argos.34.135.204.210.sslip.io 7448` checks TCP reachability
only. Confirm TLS authentication and fresh rover telemetry in the desktop app.
A failed login needs a credential check; a TLS failure needs certificate/hostname
and clock checks. Do not disable TLS validation. Keep 7449 bound to loopback.
Stopping the VM does not release its disk or reserved IP, which can incur charges.

## Operational logging

The independent collector uses HTTPS `/observability/` and loopback port7450;
see `../Terra/docs/observability/operations.md` for writer/reader provisioning,
source setup, retention, replay and rollback. The logging installer preserves the
active router and TLS material. During deployment retain the host's current
Caddy routes and add the optional observability import rather than replacing an
active dashboard route. Set `ARGOS_RELEASE_DIR` for an isolated release directory.
