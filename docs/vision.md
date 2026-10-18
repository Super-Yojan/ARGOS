# Vision and command model

!!! tip "TL;DR"
    The operator is a general at the map.
    A coarse order asks the staff to fill in the rest.
    A point named on one rover ships today as a waypoint.
    Area intent, tasking, and voice are planned.

```mermaid
flowchart TD
  op([Operator])
  intent["Broad intent\ncover this area"]
  task["Tasking\nthese units, this region"]
  point["Directed order\nthis rover, this point"]
  staff["Staff draws the plan\narea, bodies, paths"]
  accept{Accept, reject, or adjust}
  send["Send waypoint\none latched goal"]
  drive["Take over\nyou drive"]
  follower["Onboard follower\nshipped"]
  op --> intent --> staff --> accept --> follower
  op --> task --> staff
  op --> point --> send --> follower
  op --> drive --> follower
  classDef planned fill:#fff8e1,stroke:#f9a825,color:#333
  classDef shipped fill:#e8f5e9,stroke:#2e7d32,color:#333
  class intent,task,staff,accept planned
  class point,send,drive,follower shipped
```

*Amber is planned. Green is in the app. A directed point is green. Voice on that point is still amber.*

The [browser guide](browser-dashboard.md) defines current UI capabilities. Native mission proposals and reports are retained legacy workflows; their existence does not imply browser parity.

## Three grains

![Staff recommendation. An area, dashed paths, accept, reject, adjust.](design/operator-dashboard/wireframes/05-recommendation.png)

*Broad intent and tasking. The staff draws the area. You accept before anything is dispatched. Planned.*

| Grain | You do | Screen | Status |
| --- | --- | --- | --- |
| Broad intent | “Cover this.” Or draw the area | Area, bodies, ghost paths, a reason | Planned |
| Tasking | Pick the units and a region | Same drawing. You chose the units | Planned |
| Directed order | One rover, one point | A point and a confirm on that rover | The point ships as a waypoint. Voice is planned |

Take over is finer still. You drive the rover you are on.

![Directed order. One robot, one point, confirm, take over.](design/operator-dashboard/wireframes/04-command-takeover.png)

*One robot. One point. Confirm. Take over is that same robot. The point is what Send waypoint does today.*

## What Send does today

![Mac app. Waypoint mode, an active goal, 0.00 m remaining.](assets/macos-dashboard.jpg)

*Historical native console screenshot. The current browser uses a shared fleet field and selected-rover controls.*

```mermaid
sequenceDiagram
  participant You as ARGOS
  participant Bus as Zenoh
  participant Bot as Terra or Zorvane
  You->>Bus: goal + token, once
  Bus->>Bot: latch the goal
  Bot-->>You: goal/status with the same token
  Note over You: pending, then active or arrived
  You->>Bus: {"cancel":true}
  Bot-->>You: idle
```

*Published is not accepted. The matching token is acceptance. 150 seconds of silence is unconfirmed. ARGOS does not resend.*

- **Waypoint.** Local metres or WGS84. One publish on `<prefix>/<id>/goal`.
- **Cancel.** Exactly `{"cancel":true}`. Idle releases the latch. Closing the app leaves the goal in place.
- **Autonomy.** Vehicle-advertised `teleop`, `assisted_teleop`, `waypoint_direct`, `waypoint`, `target_search`, and legacy `supervised` modes. Requested level and effective level stay separate.
- **Supervised.** A `goal/proposal` does not move the robot. Approve or reject sends `goal/decision` with the proposal id and the run id.
- **Stop.** `safety` `stop` latches at Terra. `reset` is a second action.
- **Teleop.** Held input on `<prefix>/<id>/teleop`; release clears intent. Client-specific rates and limits are described in the browser and legacy guides.

Full key list: [Zenoh interfaces](zenoh.md). The scene these grains belong to: [dashboard design](design/operator-dashboard/README.md). The staff that would draw the amber boxes: [two-stage VLA](vla.md).

## Rules that stay

- Fleet supervision uses a 90-second horizon; manual/hardware actions use stricter freshness, and map display has its own 2.5-second stale threshold.
- A rover that has left the fleet cannot take a new goal.
- Local axes stay within ±20 km. Latitude [−85, 85]. Longitude [−180, 180]. Yaw within ±2π.
- Goal-status `x` and `y` are the target. The rover’s position comes from pose.
- In a geographically aligned frame, +x is north and +y is west. The browser uses validated reported frame/reference data; it does not invent an anchor.
- One browser command claim at a time. Native clients are outside that claim. Cancel has no wire token.
- L2 direct waypoint bypasses obstacles; L3 uses obstacle-aware planning on the vehicle.

Issues: [#2](https://github.com/Super-Yojan/ARGOS/issues/2) natural language and urgency. [#8](https://github.com/Super-Yojan/ARGOS/issues/8) the dashboard design.
