# Vision and command model

The operator is a general at the map. Vehicles are already in the field. An order is an objective, a tasking, or a point, and the staff fills in only what the operator left open. That picture is the [operator dashboard design](design/operator-dashboard/README.md). This page records which of those orders the app can publish today, and which grains are still planned.

## Three grains

The design names three grains. A finer order leaves less for the staff to invent. Takeover is finer still: the operator drives the robot they are looking at.

| Grain | Operator act | What should happen | Status |
| --- | --- | --- | --- |
| Broad intent | Name or draw an area (“cover this”) | Staff marks the area, the vehicles, and ghost paths, then waits for accept, reject, or adjust | Planned. No area intent and no ARGOS staff model in this repo |
| Tasking | Choose the vehicles and a region | Same drawing. The operator chose the units; the staff still asks before it adds routes and roles | Planned |
| Directed order | One vehicle and one point, or a voice order while that vehicle is in view | One point and a confirm on that vehicle. Everyone else stays on their current work | The point half is shipped as a latched waypoint. Voice, a scene confirm, and “leave everyone else” as a staff behavior are planned |

The shipping Mac and iOS app still opens on a rover list. Explicit confirm before a motion intent is published, effective authority separate from the requested level, a latched stop with a separate reset, and an exported session log are already in that app and stay in the design.

## What a command is today

ARGOS publishes one-shot and leased messages. Terra (or Zorvane, running Terra’s crates) decides whether the chassis moves. Publication is a send. A matching status is acceptance.

**Waypoint.** The operator selects Waypoint, names a local `(x, y)` in metres or a WGS84 latitude and longitude, and sends once. ARGOS generates a UUID token and publishes `<prefix>/<id>/goal`. A later `goal/status` with the same token moves the command from pending to active or arrived. Five seconds without that token shows unconfirmed. ARGOS does not resend. Cancel publishes exactly `{"cancel":true}` and waits for a later idle status. Idle releases Terra’s latched goal so debug teleop can move the rover again. Closing, disconnecting, or backgrounding the app leaves the latched goal in place.

**Autonomy.** Per rover, the operator requests `teleop`, `assisted_teleop`, `waypoint`, or `supervised`. The request carries a token. Terra’s `autonomy/status` reports requested level, effective level, safety, revision, run id, and result. The panel keeps those apart. A request stays disabled while the link is down, status is stale (2.5 seconds), or an acknowledgement is still pending. Pending operator actions show unconfirmed after two seconds.

**Supervised approval.** A `goal/proposal` is a frontier Terra is willing to pursue. The proposal itself does not authorize motion. Approve and reject send `goal/decision` with `proposal_id` and `run_id`. Resume sends `decision: resume` with the run id. Expiry, a replaced proposal, a new run, or stale telemetry drops the old decision.

**Takeover and stop.** Take over fleet, and take over one rover, clear held drive and request `teleop`. Emergency stop sends `safety` with `action: stop`. Terra latches the stop. Reset is a separate `action: reset` and does not restore the previous twist. A fleet action fans the same payload out to each known rover id and keeps each token. One rover’s acknowledgement is that rover’s.

**Held teleop.** With fresh confirmed `teleop` or `assisted_teleop` authority, safety `clear`, and a healthy link, W/S or the vertical touch pad set linear, and A/D or the horizontal pad set angular. The client publishes `<prefix>/<id>/teleop` about every 50 ms while a sample is nonzero, then a zero sample on release. Leaving the view, losing focus, changing rover, or losing authority clears the keys and publishes zero when the session is still up. The sample is −1, 0, or 1 on each axis. Speed limits and the 500 ms command lease belong to the vehicle side.

**Mission report.** `mission/report` carries a `survivor_id` the operator confirms from observed mission state. It does not open an acknowledgement wait.

Session JSONL records the manifest, goal and cancel sends, action tokens, teleop packets (session id and sequence), and observed mission and experiment state. Export flushes a snapshot. A failed recorder is visible and leaves stop available.

## Confirmation rules that stay

These rules are implemented in `argos-core` and `argos-zenoh`, and the design keeps them:

- Membership, pose, and goal status age independently. 2.5 seconds is stale. Bad packets keep the last good values and do not refresh those ages.
- Unknown or removed members cannot take a new goal. Fleet ids may have gaps. A count that disagrees with `ids` is a notice.
- Goal coordinates are finite and bounded: local axes within ±20 km, latitude in [−85, 85], longitude in [−180, 180], yaw within ±2π, token 1–64 characters from `[A-Za-z0-9._:-]`, payload at most 2048 bytes.
- Goal-status `x` and `y` are the target. Rover position comes from pose telemetry. With tiles, local +x is north and +y is west of the anchor the operator typed. The bus does not advertise that anchor.
- One operator is the assumption. Cancel has no wire token, so two operators cannot correlate a cancel. Multi-operator arbitration is outside this slice.
- Terra’s waypoint follower does not plan around obstacles.

## Planned, from the same picture

Voice, a place with robots as objects, third-person clouds, urgency painted on a body, and staff recommendations drawn on the ground are specified in the [dashboard design](design/operator-dashboard/README.md) and its [wireframes](design/operator-dashboard/wireframes/index.md). They are design only. The [two-stage VLA](vla.md) is the staff those recommendations need. Issue [#2](https://github.com/Super-Yojan/ARGOS/issues/2) tracks attention reports, urgency ranking, and natural language to commands. Issue [#8](https://github.com/Super-Yojan/ARGOS/issues/8) tracks the dashboard design.
