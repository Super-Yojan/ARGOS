---
layout: ../layouts/DocPage.astro
title: Simulation or real
description: Point ARGOS at a local Zorvane simulator to practise, or at the real fleet to drive Terra rovers. The controls are the same in both.
---

ARGOS connects to one fleet at a time. In **simulation** it talks to a Zorvane simulator on your computer. In **real** mode it talks to the rovers through the fleet's Zenoh router. The map, the inspector and every control work the same way in both, because Zorvane speaks the same topics as a real Terra rover.

| | Simulation | Real |
| --- | --- | --- |
| Option | `--sim` | `--real`, or no option |
| Window title | ARGOS · Simulation | ARGOS · Real fleet |
| Endpoint | `tcp/127.0.0.1:7447` | `ARGOS_ZENOH_ENDPOINT`, default the hosted router |
| Prefix | `terra/rover` | `ARGOS_ZENOH_PREFIX`, default `terra/phone` |
| Password | None | `ARGOS_ZENOH_PASSWORD` |

## Switch without restarting

The **Sim** | **Real** buttons in the console bar, right after the ARGOS name, switch the open window between the two. Switching lets go of any held drive keys, releases control, clears the selection, closes the old connection and its video, and connects again with the same settings the options use. It does not stop or change any rover. If the real-fleet settings are invalid, the window stays where it was and shows why.

`--sim` and `--real` still choose where a window starts. With neither, it starts on Real.

Check the window title before you arm anything. ARGOS never starts Zorvane for you and never mixes simulated and real rovers in one window.

## Simulation

1. Download [Zorvane](https://super-yojan.dev/Zorvane/running) and start a match. It listens on `tcp/127.0.0.1:7447`.
2. Open ARGOS with `--sim`.

```sh
./argos --sim
```

`--sim` overrides the endpoint, prefix, user and password, so a `.env` set up for the real fleet is left alone and its password is never sent to the simulator. Everything else in `.env` (the Maps key, the drive caps) still applies.

If Zorvane runs on another computer, start it with `TERRA_ZENOH_LISTEN=tcp/0.0.0.0:7447` and point ARGOS at it with the real-mode settings below, using an empty user and password.

## Real fleet

Real mode reads the connection from `.env` or the environment. The built-in defaults are the hosted fleet, so all most operators need is the password:

```sh
ARGOS_ZENOH_PASSWORD=the-fleet-password
```

Ask the fleet operator for it, put it in `.env` ([where ARGOS looks](/ARGOS/running#the-env-file)), and open ARGOS with no option or with `--real`.

Without a password the window still opens and a notice on the map says to set it. A router that is down does not stop the window either; the console bar shows the link state.

### A router on your network

To drive rovers through a Zenoh router on the LAN instead of the hosted one, set all four connection values. An empty user and password mean the router has no authentication:

```sh
ARGOS_ZENOH_ENDPOINT=tcp/terra1.local:7447
ARGOS_ZENOH_PREFIX=terra/rover
ARGOS_ZENOH_USER=
ARGOS_ZENOH_PASSWORD=
```

A bad endpoint, one that is not `tcp/` or `tls/`, or a user without a password on a `tcp/` router prints the reason and exits before the window opens.
