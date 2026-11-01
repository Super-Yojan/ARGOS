# What you can read

ARGOS is subscribed to every rover's Zenoh topics (`{prefix}/{id}/...`, prefix
`terra/phone` on the real fleet and `terra/rover` in the Zorvane simulator).
You read them through ARGOS, not Zenoh directly, so sim and real look the same.

## `get_field_state` (also in every tick's prompt)

| Field | From topic | Meaning |
| --- | --- | --- |
| `rovers[].pose` | `pose`, `pose/quality`, `localization`, `navigation/outdoor` | Local x/y/yaw, latitude/longitude, degraded flag |
| `rovers[].level` | `autonomy/status` | Requested and effective level, study condition |
| `rovers[].safety`, `stopped`, `hold`, `reason` | `autonomy/status` | Stop latch, holds and Terra's reason |
| `rovers[].armed`, `drive_block` | `hardware/status` | Motors armed, why driving is blocked |
| `rovers[].navigation` | `navigation/outdoor` | `local` or `outdoor_gps` |
| `rovers[].goal`, `goal_active` | `goal/status`, `autonomy/status` | Current goal and its state (`active`, `arrived`) |
| `rovers[].exploration` | `autonomy/status` (`exploration`) | L4 run phase, time left, coverage, end reason |
| `rovers[].task` | `autonomy/status` (`task`) | L5 task phase, question for the operator, time left |
| `rovers[].objects` | `objects` | Confirmed buckets and balls: id, class, color, x/y, latitude/longitude |
| `unlocated` | `fleet/state` | Rovers in the fleet without a position yet |
| `recommendations` | this station | Open cards and recent decisions with reasons |
| `drive_grant` | this station | A raw-drive request and whether it was approved |
| `operator` | this station | `has_control` (an Operator-mode command takes it), selected rover, whether the operator is driving, the last note |

## `read_rover(rover)`

The raw cached JSON for one rover: `authority` (autonomy/status), `hardware`,
`outdoor`, `pose_quality`, `objects`, `search` and its reports, `experiment` and
`mission`, each with its `age` in seconds. Use it when a field above is not
enough.
