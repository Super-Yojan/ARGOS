---
layout: ../layouts/DocPage.astro
title: Settings
description: Every variable ARGOS reads from .env or the environment. The environment wins over any file.
---

Set these in a `.env` file ([where ARGOS looks](/ARGOS/running#the-env-file)) or in the environment. A downloaded build carries `.env.example` with each one commented.

## Connection

In simulation (`--sim`, or **Sim** in the console bar) ARGOS ignores these four and uses the local Zorvane values. See [Simulation or real](/ARGOS/modes).

| Variable | Default | Role |
| --- | --- | --- |
| `ARGOS_ZENOH_ENDPOINT` | `tls/argos.34.135.204.210.sslip.io:7448` | The fleet router. `tcp/` or `tls/` |
| `ARGOS_ZENOH_PREFIX` | `terra/phone` | Key prefix. Zorvane and LAN routers use `terra/rover` |
| `ARGOS_ZENOH_USER` | `terra` | Router user. Empty for a router with no authentication |
| `ARGOS_ZENOH_PASSWORD` | unset | The fleet password. Never committed or printed |

`tls/` uses the system trust store.

## Map

| Variable | Default | Role |
| --- | --- | --- |
| `GOOGLE_MAPS_API_KEY` | unset | Photorealistic 3D Tiles. Without it a notice on the map asks for it |
| `MAPS_DEMO` | unset | `1` draws an offline fixture and never calls Google. Read from the environment only, not `.env` |
| `ARGOS_MAP_START_LAT`, `ARGOS_MAP_START_LON` | 38.8297, -77.3075 | Where the camera opens, until a rover reports a position |
| `ARGOS_MAP_ORIGIN_LAT`, `ARGOS_MAP_ORIGIN_LON` | the start pair | Origin for a rover's local metres when it has not published its own |

## Drive caps

These cap what this window sends. The rover enforces its own limits as well.

| Variable | Default | Role |
| --- | --- | --- |
| `ARGOS_DRIVE_LINEAR` | 0.2 | Speed cap, 0 to 0.5 m/s, forward and reverse |
| `ARGOS_DRIVE_YAW_RATE` | 0.5 | Turn-rate cap, up to 1 rad/s |
| `ARGOS_DRIVE_YAW_ACCEL` | 0.5 | How fast the turn rate may rise, rad/s² |
| `ARGOS_DRIVE_YAW_DECEL` | 0.8 | How fast the turn rate may fall, rad/s² |
| `ARGOS_DRIVE_MIN_RADIUS` | 0.5 | Smallest turn radius while rolling, metres |
| `ARGOS_DRIVE_ALLOW_SPIN` | off | `1`, `true` or `yes` allows turning on the spot |
| `ARGOS_DEGRADED_YAW_RATE` | 0.4 | Turn rate that raises a pose warning when the rover sends no pose quality |
| `ARGOS_DEGRADED_SETTLE` | 1.0 | How long that warning stays up, seconds |

## Video

| Variable | Default | Role |
| --- | --- | --- |
| `ARGOS_FPV_STUN` | `stun:stun.l.google.com:19302` | STUN URLs, comma-separated. `none` keeps to local addresses |
| `ARGOS_FPV_TURN_URL` | unset | `turn:` or `turns:` URL, with no credentials in it |
| `ARGOS_FPV_TURN_USERNAME` | unset | TURN user, set together with the URL and credential |
| `ARGOS_FPV_TURN_CREDENTIAL` | unset | TURN password. Never committed or printed |

A number that does not parse, or a connection that cannot work, prints the reason and exits before the window opens.
