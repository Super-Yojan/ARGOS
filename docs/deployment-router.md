# Hybrid fleet router

ARGOS Mac can host an app-owned router through Connection settings → Local on this Mac. Connect starts it before the fleet session. It listens on `tcp/127.0.0.1:7448`, with a choice of loopback only, Local network (`0.0.0.0`), or an explicitly entered Mac Tailscale IPv4 address. Local network shares on every interface and advertises the Mac router through Bonjour. Disconnect leaves the router running for other clients. Stopping it during a connection requires confirmation. Router logs and bind failures appear in settings.

For an independently running router, use Remote host, even on the same Mac. ARGOS never adopts or stops that service. An occupied local port reports a bind failure; select Remote host with `tcp/127.0.0.1:7448` to reuse it.

For LAN use, choose Find LAN routers in Remote host, or enter `tcp/<Mac LAN IP>:7448`; Tailscale is unnecessary. Allow local-network access when prompted. Bonjour discovery applies to Mac app-owned LAN routers; standalone hosts can be reached by address. Use LAN sharing only on a trusted network.

For Tailscale on iPad select Remote host and enter `tcp/<router Tailscale IP>:7448`. Phones publishing fleet data use that same host/port and matching prefix, normally `terra/phone`. Enable Tailscale on all devices and permit TCP 7448 in tailnet policy. Simulator connections can use their existing `tcp/127.0.0.1:7447` endpoint with prefix `terra/rover` on the Mac.

## Standalone host

Run `sh scripts/build-router.sh`, then `target/release/argos-router --bind 100.x.y.z`. Omit the address for loopback-only use. `--port` overrides 7448. `--bind 0.0.0.0` enables LAN/all-interface sharing; it has no client authentication, so do not expose this port to the public internet. Stop a standalone process through its process manager; the app does not own it.

For Linux install the release executable at `/usr/local/bin/argos-router`, install `packaging/router/argos-router.service`, and create `/etc/argos/router.env` containing `ARGOS_ROUTER_BIND=100.x.y.z`, or `ARGOS_ROUTER_BIND=0.0.0.0` for LAN. Enable the service with systemd. A cloud VM uses the same setup after joining the tailnet; no public TCP router port is required.

Container: `docker build -f packaging/router/Dockerfile -t argos-router .`. On a Linux host already running Tailscale, run with host networking: `docker run --rm --network host argos-router --tailscale-address 100.x.y.z`. Docker Desktop cannot bind the host's Tailscale address inside its VM; use the native Mac router there. Container compilation/package definition is supplied; production cloud provisioning is separate.

Changing connection profiles closes the old ARGOS session and clears its pending command and scene state. A router restart or reconnection never replays motion commands. Closing ARGOS ends its in-process local router; use standalone hosting when other clients must stay connected without the app.
