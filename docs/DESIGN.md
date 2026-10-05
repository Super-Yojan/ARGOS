# ARGOS fleet supervision

ARGOS (Adaptive Robotic Group Operator System) supervises a Terra rover fleet over Zenoh. Terra (https://github.com/Super-Yojan/Terra) is a separate repository. This repo does not contain Terra code.

Tracking board: [ARGOS project #6](https://github.com/users/Super-Yojan/projects/6). Terra issue #5 moved this work out of the Terra repo.

## End state

Terra runs local autonomy onboard. Each rover senses, plans, and drives itself. ARGOS stays at supervision range: it learns who is in the fleet, watches health, and sends high-level commands. Those commands are missions, goals, and fleet intents. Terra's onboard stack closes the loop.

The long-term operator contract is that high-level command set. ARGOS publishes a mission, a goal, or a fleet intent and then lets Terra carry it out. Continuous low-level teleoperation is outside that contract.

## Target boundary

| Side | Owns |
| --- | --- |
| Terra | Sensing, local planning, the command watchdog, low-level drive, onboard autonomy, and the Zenoh topic contract |
| ARGOS | Fleet supervision and high-level commands over Zenoh: missions, goals, and fleet intents |

A mission tells a rover, or the fleet, what to accomplish (inspect a region, return to base, hold). A goal is an objective Terra's local planner closes, such as a pose. A fleet intent is a group-level request. Desired fleet size is the one intent Terra already accepts, on `fleet/size`. Further mission and goal keys are part of the target contract and are not on the bus yet. They land in `argos.contract` only after Terra specifies the key names and payloads.

ARGOS does not take over sensing, local planning, the watchdog, or the motor path. A later UI can call the same supervisor the CLI uses.

## This repository today

The working slice is a spike on the topics Terra already publishes. It discovers rover ids, shows derived health, and can request a fleet size. It also includes a `cmd_vel` twist command so an operator can prove the Zenoh session and see a rover move.

That twist path is a temporary first-slice debug tool. It is the way this spike talks to today's simulator. It is not the long-term operator contract. Streaming twists at 20 Hz keeps ARGOS inside Terra's 500 ms command watchdog and bypasses the onboard autonomy the end state assumes. Leave the code in place until a Terra mission or goal topic replaces it. New operator features should extend supervision and high-level commands, and should leave `cmd_vel` as debug.

`fleet/size` is already the right kind of message: one fleet intent, acknowledged by `fleet/state`, with Terra deciding how to spawn and remove rovers.

## Spike topics

Default prefix `terra/rover`. Override with `--prefix` / `ARGOS_ZENOH_PREFIX` when Terra is started with `TERRA_ZENOH_PREFIX`. Names for this spike are built only in `argos.contract.TerraTopics`.

Terra's current bus is documented in [simulator/ZENOH.md](https://github.com/Super-Yojan/Terra/blob/main/simulator/ZENOH.md) and `simulator/src/zenoh_bridge.rs`.

| Key | Direction | Payload | Role in this spike |
| --- | --- | --- | --- |
| `<prefix>/fleet/state` | Terra → ARGOS | `{"count":3,"max_count":32,"ids":[0,1,2]}` | Supervision. Discover ids and fleet size. Terra publishes on change and about once a second. |
| `<prefix>/fleet/size` | ARGOS → Terra | `{"count":3}` | Fleet intent already on the bus. Integer `0` through `32`. Terra ignores any other body. |
| `<prefix>/<id>/cmd_vel` | ARGOS → Terra | `{"linear":1.0,"angular":0.3}` | Temporary debug twist. `linear` is m/s forward, `angular` is rad/s left. Finite f32 values, exactly those two fields, at most 2048 bytes. |

There is no shared twist topic. Commands for an id that is not spawned are ignored by Terra. The debug path repeats a twist at 20 Hz and publishes a zero twist when the drive interval ends, matching Terra's Python client. Terra drops the command 500 ms after the last valid sample.

These Terra keys are unused by the spike:

| Key | Why it stays on Terra's side |
| --- | --- |
| `<prefix>/<id>/camera/rgb` | Sensing product for Terra, and later a supervision display |
| `<prefix>/<id>/camera/depth` | Sensing product for Terra, and later a supervision display |

Session shape matches Terra's client: Zenoh client mode, connect to `tcp/127.0.0.1:7447` (`TERRA_ZENOH_LISTEN` on the simulator), multicast scouting off.

## Spike operator actions

1. **Discover.** Subscribe to `fleet/state`. Report `count`, `max_count`, and `ids`. Ids can have gaps; `count` is how many rovers exist, not the highest id.
2. **Show health.** Print link freshness and, for the debug twist path, per-rover motion. Terra does not publish a separate health topic. Long-term health should describe mission progress and rover condition, which Terra will report once that contract exists.
3. **Resize.** Wait up to one second for a `fleet/state` sample, publish `fleet/size`, and wait until `fleet/state.count` equals the request. Same acknowledgement rule as Terra's `zenoh_client.py`. The brief wait lets a shrink show which ids left. This action is a fleet intent and stays in the target contract.
4. **Debug drive.** `argos drive` publishes `cmd_vel` for one id, then a zero twist. It warns when that id is missing from the latest fleet set. Use it to check the link. It is the temporary spike described above.

## Health model in the spike

| Field | Values | Rule |
| --- | --- | --- |
| `link` | `unknown`, `healthy`, `stale` | `unknown` until the first valid `fleet/state`. `stale` when that sample is 2.5 s old or older, which is longer than two of Terra's one-second republishes. |
| rover `health` | `online`, `absent`, `stale`, `unlisted` | `online` ids are in the latest healthy sample. `absent` ids were seen earlier in this process and are missing from the latest sample; a repeated sample does not clear them, and the id becomes `online` again if it returns. `stale` ids are the last reported set while the link is stale. `unlisted` ids have a local debug twist and have never appeared in `fleet/state`. |
| rover `motion` | `driving`, `idle` | Debug-path only. `driving` while this process's last non-zero twist is younger than 500 ms. Otherwise `idle`, including after ARGOS sends the stopping zero twist. |

A rover can be `online` and `idle` while some other client is still sending twists. This process only knows twists it sent. That motion field goes away with the debug path. Supervision health remains.

Invalid `fleet/state` payloads do not replace the last good sample. A `count` that disagrees with `len(ids)` is still shown, with a notice. Membership follows `ids`.

## Out of scope for the spike

- The mission and goal wire format (target contract, waiting on Terra)
- Dashboard or web UI
- Camera frames, occupancy maps, odometry (odometry is not on this bus)
- Local planning, task assignment, and multi-operator arbitration
- Motor PWM, enable, and the hardware watchdog (Terra `terra-motors`)
- iOS / TerraPhone transport
- Reproducing Terra's id-retirement rules inside the mock peer

The mock peer (`python -m argos.mock_fleet`) stands in for `fleet/state`, `fleet/size`, and the debug `cmd_vel` key so the CLI can run without the simulator. After a shrink, Terra keeps surviving ids and may leave gaps. The mock renumbers from zero.

## Where to change the contract

Edit `src/argos/contract.py` if Terra renames a key, the prefix, the 32-rover request cap, or the JSON fields. Supervisor and CLI code should keep calling `TerraTopics`, `encode_twist`, `encode_fleet_size`, and `decode_fleet_state`.

Add mission and goal codecs in that same module when Terra defines them. Until then, `encode_twist` remains the debug adapter and should stay marked as temporary.
