# ARGOS fleet supervision — first slice

ARGOS (Adaptive Robotic Group Operator System) is the operator side of a Terra fleet. Terra owns the simulator, the rovers, and the Zenoh contract. ARGOS discovers rovers, sends twists, and shows basic health. This repo does not contain Terra code.

Tracking board: [ARGOS project #6](https://github.com/users/Super-Yojan/projects/6).

Terra contract used here: [simulator/ZENOH.md](https://github.com/Super-Yojan/Terra/blob/main/simulator/ZENOH.md) and `simulator/src/zenoh_bridge.rs`. Terra issue #5 moved this slice out of the Terra repo.

## Boundary

| Side | Owns |
| --- | --- |
| Terra | Spawned rovers, physics, command watchdog, topic names, payload rules |
| ARGOS | Operator actions, a thin topic adapter, derived health, CLI |

The CLI is the operator surface for this slice. A later UI can call `FleetSupervisor` the same way. ARGOS does not render cameras, plan paths, or drive motors.

## Topics

Default prefix `terra/rover`. Override with `--prefix` / `ARGOS_ZENOH_PREFIX` when Terra is started with `TERRA_ZENOH_PREFIX`. Names are built only in `argos.contract.TerraTopics`.

| Key | Direction | Payload | ARGOS use |
| --- | --- | --- | --- |
| `<prefix>/fleet/state` | Terra → ARGOS | `{"count":3,"max_count":32,"ids":[0,1,2]}` | Discover ids and fleet size. Terra publishes on change and about once a second. |
| `<prefix>/fleet/size` | ARGOS → Terra | `{"count":3}` | Optional resize. Integer `0` through `32`. Terra ignores any other body. |
| `<prefix>/<id>/cmd_vel` | ARGOS → Terra | `{"linear":1.0,"angular":0.3}` | Twist for one rover. `linear` is m/s forward, `angular` is rad/s left. Finite f32 values, exactly those two fields, at most 2048 bytes. |

There is no shared twist topic. Commands for an id that is not spawned are ignored by Terra. ARGOS repeats a twist at 20 Hz and publishes a zero twist when the drive interval ends, matching Terra's Python client. Terra drops the command 500 ms after the last valid sample.

These Terra keys are intentionally unused here:

| Key | Why it is out of this slice |
| --- | --- |
| `<prefix>/<id>/camera/rgb` | Frames, not fleet supervision |
| `<prefix>/<id>/camera/depth` | Frames, not fleet supervision |

Session shape matches Terra's client: Zenoh client mode, connect to `tcp/127.0.0.1:7447` (`TERRA_ZENOH_LISTEN` on the simulator), multicast scouting off.

## Operator actions

1. **Discover.** Subscribe to `fleet/state`. Report `count`, `max_count`, and `ids`. Ids can have gaps; `count` is how many rovers exist, not the highest id.
2. **Drive.** Publish `cmd_vel` for one id, then a zero twist. Warn when that id is missing from the latest fleet set.
3. **Show health.** Print link freshness and per-rover motion. This is derived. Terra does not publish a health topic.
4. **Resize (pass-through).** Publish `fleet/size` and wait until `fleet/state.count` equals the request. Same acknowledgement rule as Terra's `zenoh_client.py`.

## Health model

| Field | Values | Rule |
| --- | --- | --- |
| `link` | `unknown`, `healthy`, `stale` | `unknown` until the first valid `fleet/state`. `stale` when that sample is 2.5 s old or older, which is longer than two of Terra's one-second republishes. |
| rover `health` | `online`, `absent`, `stale`, `unlisted` | `online` ids are in the latest healthy sample. `absent` ids were in the previous healthy sample and dropped out. `stale` ids are the last set while the link is stale. `unlisted` ids have a local twist and are not in that set. |
| rover `motion` | `driving`, `idle` | `driving` while this process's last non-zero twist is younger than 500 ms. Otherwise `idle`, including after ARGOS sends the stopping zero twist. |

A rover can be `online` and `idle` while some other client is still commanding it. ARGOS only knows twists it sent.

Invalid `fleet/state` payloads do not replace the last good sample. A `count` that disagrees with `len(ids)` is still shown, with a notice. Membership follows `ids`.

## Out of scope

- Dashboard or web UI
- Camera frames, occupancy maps, odometry (odometry is not on this bus)
- Path planning, task assignment, multi-operator arbitration
- Motor PWM, enable, and the hardware watchdog (Terra `terra-motors`)
- iOS / TerraPhone transport
- Reproducing Terra's id-retirement rules inside the mock peer

The mock peer (`python -m argos.mock_fleet`) only stands in for the three keys above so the CLI can be exercised without the simulator. After a shrink, Terra keeps surviving ids and may leave gaps. The mock renumbers from zero.

## Where to change the contract

Edit `src/argos/contract.py` if Terra renames a key, the prefix, the 32-rover request cap, or the JSON fields. Supervisor and CLI code should keep calling `TerraTopics`, `encode_twist`, `encode_fleet_size`, and `decode_fleet_state`.
