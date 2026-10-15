# Public ARGOS host

Deployed 2026-10-15 in GCP project `glid-app`:

- VM `argos-host`, zone `us-central1-a`, `e2-small` (2 GB RAM), Debian 12, 20 GB standard persistent disk.
- Static address `argos-public-ip`: `34.135.204.210`.
- Dashboard: https://argos.34.135.204.210.sslip.io
- Fleet: `tls/argos.34.135.204.210.sslip.io:7448`, prefix `terra/phone`.
- Public ports: 80/443 for Caddy HTTPS, 7448 for authenticated Zenoh TLS. No VPN required.
- Backend binds `127.0.0.1:8080`; internal fleet listener binds `127.0.0.1:7449`.
- HTTPS uses the IP-derived sslip.io hostname and Caddy-managed public certificates. Keep the static IP reserved; a custom DNS name can replace this later.

## Terra

The sibling Terra app defaults to this TLS router, preserving a custom saved endpoint. In **Settings → Fleet connection**, use account `terra` and enter `ARGOS_ZENOH_PASSWORD` from the ignored local `packaging/gcp/.env` file. On the VM it is in root-only `/etc/argos/fleet.password`. The iPhone stores it in Keychain. Prefix is `terra/phone`; give each simultaneously connected rover a distinct numeric rover ID.

Do not embed the fleet password in an app binary, source control or a public frontend. Connecting the fleet transport does not arm hardware. Rebuild/reinstall Terra to pick up the changed defaults and Rust bindings.

## Authentication and data

GitHub OAuth uses public profile identity only; GitHub numeric ID `33645913` is the initial operator. Add collaborators' numeric IDs to `ARGOS_VIEWERS` or `ARGOS_OPERATORS` in `/etc/argos/server.env` and restart `argos-server`. Client ID and secret live in that root-only environment file. Callback: `https://argos.34.135.204.210.sslip.io/auth/callback`.

Fleet credentials are a shared fleet membership secret, separate from browser roles. All authenticated fleet clients can exchange fleet topics. Issue distinct router dictionary entries if individual membership revocation is needed. Keep credentials confidential.

SQLite state/audit data lives in `/var/lib/argos/argos.db`. All services restart after failure/reboot. Caddy renews certificates; the TLS timer copies renewed certificates for the router and restarts it when changed. This interrupts transport temporarily, requiring fresh observations before commands.

## Operate

```sh
gcloud compute ssh argos-host --project=glid-app --zone=us-central1-a
sudo systemctl status caddy argos-router argos-server
sudo journalctl -u argos-server -u argos-router --since '10 minutes ago'
```

Live `/api/v1/events` responses bypass compression to prevent proxy buffering. The browser reopens stalled streams after three seconds and clears unsubmitted drafts; it never replays a vehicle command.

`/healthz` checks HTTP availability. `/readyz` requires fresh fleet telemetry; **503 is expected with no active phones**, even when the router connection is established.

## Deploy another release

Build the web bundle with `npm ci --prefix apps/web && npm run build --prefix apps/web`. Build Linux binaries with:

```sh
rustup target add x86_64-unknown-linux-gnu
cargo zigbuild --release --target x86_64-unknown-linux-gnu \
  -p argos-server -p argos-zenoh --bin argos-server --bin argos-router
```

Create `/tmp/argos-release/{bin,web,gcp}` containing the two binaries, web `dist` contents and these GCP packaging files. Transfer it to the VM, then run `sudo bash /tmp/argos-release/gcp/install.sh`. The installer preserves existing credentials. On first install, supply `GITHUB_CLIENT_ID` and `GITHUB_CLIENT_SECRET` in `/etc/argos/server.env` before starting `argos-server`. After Caddy obtains its certificate, run `sudo systemctl start argos-router-tls.service` to initialize fleet TLS immediately; otherwise the timer retries within five minutes.

VM, disk, reserved public IP and network usage incur GCP charges. Stopping the VM does not release the disk or reserved IP.
