# Roadmap

The live board is [ARGOS — Adaptive Robotic Group Operator System](https://github.com/users/Super-Yojan/projects/6). Priority on that board is a field (P0, P1, P2). The table below is a snapshot of the 19 items on the board as of 2026-10-08. The board is the source of truth if they move.

The board’s own readme splits the work into five phases. Zorvane is the world simulator extracted from Terra ([Terra #23](https://github.com/Super-Yojan/Terra/issues/23)); the board still tracks that extraction on the Terra repo.

| Phase | Intent |
| --- | --- |
| 0 · Sim MVP | Zenoh drive, motor adapter, ARGOS fleet supervision. Done |
| 1 · Hardware bring-up | Phone mount, TerraPhone to the rover over Bluetooth, operator dashboard and autonomy controls |
| 2 · Shared world model | Occupancy over Zenoh, real-world tiles, go-to-waypoint, Zorvane extraction, richer vehicle bodies |
| 3 · Operator intelligence | Perception and attention reports, urgency, natural language to commands, the dashboard scene |
| 4 · Onboard autonomy | On-device VLA on TerraPhone |

## Board

| Status | Phase | Priority | Item |
| --- | --- | --- | --- |
| Done | 0 · Sim MVP | P0 | [Terra #1](https://github.com/Super-Yojan/Terra/issues/1) Motor adapter (PWM, enable, watchdog) |
| Done | 0 · Sim MVP | P0 | [Terra #2](https://github.com/Super-Yojan/Terra/issues/2) iOS Zenoh client for cmd_vel |
| Done | 0 · Sim MVP | P1 | [Terra #5](https://github.com/Super-Yojan/Terra/issues/5) ARGOS fleet-supervision MVP |
| Done | 1 · Hardware bring-up | P0 | [Terra #3](https://github.com/Super-Yojan/Terra/issues/3) Phone mount calibration checklist |
| In progress | 1 · Hardware bring-up | P0 | [Terra #10](https://github.com/Super-Yojan/Terra/issues/10) TerraPhone ↔ real hardware over Bluetooth |
| Done | 1 · Hardware bring-up | P1 | [Terra #17](https://github.com/Super-Yojan/Terra/issues/17) Level-of-autonomy arbiter and experiment logging |
| Done | 1 · Hardware bring-up | P1 | [Terra #25](https://github.com/Super-Yojan/Terra/issues/25) TerraPhone: simulator connection on Simulator, Bluetooth on device |
| Done | 1 · Hardware bring-up | P1 | [ARGOS #3](https://github.com/Super-Yojan/ARGOS/issues/3) MVP operator dashboard |
| Done | 1 · Hardware bring-up | P1 | [ARGOS #4](https://github.com/Super-Yojan/ARGOS/issues/4) Per-rover autonomy switch and takeover |
| Done | 2 · Shared world model | P1 | [Terra #4](https://github.com/Super-Yojan/Terra/issues/4) Occupancy over Zenoh |
| Done | 2 · Shared world model | P1 | [Terra #9](https://github.com/Super-Yojan/Terra/issues/9) Real-world tiles and ARGOS go-to-waypoint |
| In review | 2 · Shared world model | P1 | [Terra #21](https://github.com/Super-Yojan/Terra/issues/21) NEXT × Zatara competition field |
| Backlog | 2 · Shared world model | P1 | [Terra #22](https://github.com/Super-Yojan/Terra/issues/22) Easy robot import with flexible actuators |
| Backlog | 2 · Shared world model | P1 | [Terra #23](https://github.com/Super-Yojan/Terra/issues/23) Extract the simulator into Zorvane |
| Backlog | 3 · Operator intelligence | P1 | [Terra #11](https://github.com/Super-Yojan/Terra/issues/11) Perception reports and human-attention signals |
| Backlog | 3 · Operator intelligence | P1 | [ARGOS #2](https://github.com/Super-Yojan/ARGOS/issues/2) Attention, urgency, natural language to commands |
| Backlog | 4 · Onboard autonomy | P2 | [Terra #12](https://github.com/Super-Yojan/Terra/issues/12) On-device phone VLA |
| Backlog | — | — | [ARGOS #8](https://github.com/Super-Yojan/ARGOS/issues/8) Operator dashboard design |
| Backlog | — | — | [Terra #27](https://github.com/Super-Yojan/Terra/issues/27) TerraPhone tap-to-configure |

## Where the docs site sits

[ARGOS #9](https://github.com/Super-Yojan/ARGOS/issues/9) is the documentation pass for ARGOS, Terra, TerraPhone, and Zorvane. This site is the ARGOS part and the hub:

- [Terra](https://super-yojan.dev/Terra/) — vehicle body, onboard autonomy, Pi
- [TerraPhone](https://super-yojan.dev/Terra/terraphone/) — the iOS app section of the Terra site
- [Zorvane](https://super-yojan.dev/Zorvane/) — the world simulator

The personal site is [super-yojan.dev](https://super-yojan.dev).

Shipped operator behavior is the [command model](vision.md) and the [Zenoh keys](zenoh.md). The scene, the three command grains, and the [two-stage VLA](vla.md) are the phase 3 and phase 4 work, and they are planned. LAN deployment is current; [AWS via a tunnel](deployment.md) is planned.
