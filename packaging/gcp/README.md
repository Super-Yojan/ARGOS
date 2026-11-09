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

## Zorvane online rooms

Zorvane players browse, host and join rooms through this router. They sign in as
`zorvane` with the password in root-only `/etc/argos/zorvane.password`, which the
installer creates and adds to `users.txt` when the account is missing. Share that
password with players; it is not the fleet password.

`router.json5` turns on Zenoh access control with default deny. The `zorvane`
account may only publish, subscribe and query under `zorvane/**`, so a player can
never reach `terra/phone/**`. Every other account in `users.txt` keeps full
access. The installer builds that list from `users.txt`, so rerun it after you add
an account, or the new account can do nothing. Each room also has its own room
password, checked by the hosting simulator.

To change the player password, edit the `zorvane:` line in `users.txt` and
`zorvane.password` together, then `sudo systemctl restart argos-router`.

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
fleet credentials and custom user dictionary entries, adds the `zorvane` player
account and access control (see above), installs the router, and
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

## FPV relay (TURN)

FPV video is WebRTC straight from the phone to the Mac; only the call setup
goes through the router. When the two cannot reach each other directly
(campus or cellular networks, strict NAT, or TerraPhone without iOS Local
Network permission) the phone reports the call failed after about 15 s. A
coturn relay on this VM carries the video in that case.

From your Mac, signed in to gcloud:

```sh
bash packaging/gcp/deploy-turn.sh
```

It creates the `argos-turn` firewall rule (UDP/TCP 3478, TCP 5349, UDP
49160-49200) on VMs tagged `argos-turn`, tags `argos-host`, copies
`install-turn.sh` and runs it. The installer is safe to rerun; it keeps the
password in root-only `/etc/argos/turn.password` and only relays to public
addresses. `refresh-router-tls.sh` restarts coturn when the certificate changes.

Then set the same relay in both apps:

- ARGOS `.env`: `ARGOS_FPV_TURN_URL=turn:argos.34.135.204.210.sslip.io:3478`,
  `ARGOS_FPV_TURN_USERNAME=argos-fpv`, `ARGOS_FPV_TURN_CREDENTIAL=<turn.password>`.
- TerraPhone: fleet connection settings, "FPV video" section: same TURN URL,
  username and credential, then Save.

Check it with `sudo systemctl status coturn` and `sudo journalctl -u coturn`
on the VM. A working relayed call logs an allocation for user `argos-fpv`.

## Operational logging

The independent collector uses HTTPS `/observability/` and loopback port7450;
see `../Terra/docs/observability/operations.md` for writer/reader provisioning,
source setup, retention, replay and rollback. The logging installer preserves the
active router and TLS material. During deployment retain the host's current
Caddy routes and add the optional observability import rather than replacing an
active dashboard route. Set `ARGOS_RELEASE_DIR` for an isolated release directory.
