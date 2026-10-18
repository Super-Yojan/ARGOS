# Outdoor GPS waypoints with local obstacle avoidance

Status: proposed design for review. GPS display is already implemented and deployed;
outdoor GPS motion control is not implemented or enabled by this document.

## Intended behavior

The operator selects an outdoor navigation mode, reviews a latitude/longitude
destination on ARGOS, explicitly arms the rover, and confirms the waypoint. GPS
determines progress toward that destination. Compass heading determines the
direction to travel. ARKit and LiDAR continue to supply local motion estimation,
ground-relative occupancy and obstacle avoidance. Indoor/local waypoints remain
available through the existing mode.

There is no persistent GPS-to-AR origin, global occupancy transform, or eight-second
alignment check. Raw GPS display stays independent of navigation readiness.

## Existing constraints

- Terra's `MobileController::step` passes AR pose, a local occupancy map and
  velocity estimates to the autonomy arbiter. Pose and map share the AR frame.
- L3 `waypoint` checks sensor and map freshness and uses the local route planner.
  L2 `waypoint_direct` intentionally bypasses obstacle avoidance.
- The iOS hardware adapter currently keeps L3 disarmed without compatible wheel
  feedback. This requirement remains; this feature does not enable open-loop L3.
- The phone's latest observed GPS uncertainty was approximately 4 m. A local
  follower's sub-metre arrival threshold cannot be reused for GPS arrival.
- The deployed ARGOS service currently treats raw GPS as observation and rejects
  geographic driving commands. Outdoor commands must be capability-gated before
  that restriction can be lifted for updated vehicles.

## Global guidance and local planning

Terra owns an outdoor mission containing the reviewed WGS84 destination, command
token, rover run, navigation revision and arrival radius. It computes GPS distance
and bearing to that immutable destination. The existing local planner owns the
short path and velocity command around observed obstacles.

For a local planning update:

1. Compute GPS distance and true-north bearing to the mission destination.
2. Obtain rover true heading from the compass's configured reference axis and
   the calibrated phone-to-rover mount. Pair the compass and AR reference axes;
   do not equate the compass right-edge heading with rover forward.
3. Subtract current true heading from target bearing to obtain a body-relative
   steering direction.
4. Place a guidance target up to 3 m ahead in that direction from the current AR
   pose, using the AR rover yaw. Update this target at most twice per second.
5. Let the existing L3 planner find an observed, collision-free local path toward
   that target. Existing unknown-cell, footprint, clearance and braking checks
   remain authoritative. If the observed map cannot support progress, hold.

This is an instantaneous steering calculation. GPS position never translates or
rotates the local occupancy map, estimator pose or accumulated AR trajectory.
GPS jumps therefore cannot teleport obstacles. The local planner does not assume
that map tiles, roads or unobserved ground describe traversable space.

A temporary guidance target is distinct from the mission destination. Updating it
must preserve the mission token, acknowledgement and operation history. Reaching
it must never report the GPS mission as arrived. Route planning retains its
existing rate limits rather than running on every camera/control tick.

## Mode and capability contract

Add explicit navigation sources `local` and `outdoor_gps`. Local remains the
default. Source changes require a disarmed rover and no active mission; they are
bound to the reviewed run and navigation revision and require vehicle acknowledgement.
Selecting a source never arms hardware or starts a mission.

Terra advertises a versioned outdoor capability and status through its navigation
telemetry. Status includes source, readiness/hold reason, GPS and heading source
ages and accuracies, arrival radius, active geographic destination, remaining
distance and mission phase. Older vehicles remain local-only. ARGOS must show
an app-update-required explanation for unsupported vehicles.

Outdoor GPS missions require L3 `waypoint`. ARGOS must not silently select L2 if
outdoor avoidance is unavailable. Local missions retain existing local behavior.
The geographic API is enabled only when the selected vehicle has acknowledged
outdoor mode and reports the required capability and current readiness.

## Admission and runtime limits

Proposed initial outdoor defaults:

| Input or limit | Requirement |
| --- | --- |
| GPS observation | Finite valid WGS84 coordinates; source age at most 3 s |
| GPS uncertainty | At most 10 m and no larger than the reviewed arrival radius |
| True heading | Valid; source age at most 2 s; uncertainty at most 30 degrees |
| Local pose and occupancy | Existing L3 freshness, tracking and validity checks |
| Motor feedback | Existing L3 hardware-feedback requirement |
| Outdoor forward speed | At most 0.3 m/s and within existing lower limits |
| Mission distance at admission | At most 500 m from the current GPS fix |
| Arrival radius | Default 5 m; operator-configurable from 2 through 20 m |
| Arrival confirmation | Three distinct fresh GPS fixes inside the reviewed radius |

The arrival radius is frozen when the waypoint is reviewed. Worsening accuracy
does not silently expand the radius or complete the mission. Duplicate or replayed
observations do not count as distinct arrival fixes. Motion is zero while arrival
confirmation is pending. A valid fix outside the radius resets that confirmation
and returns the still-active mission to local planning; it does not count as a
completed waypoint. GPS and heading carry their
original observation timestamps; publishing cached samples does not renew freshness.
Platform timestamps are converted to Terra's monotonic clock at ingestion.

GPS/heading rejection or AR tracking loss stops motion and pauses the outdoor
mission. Resumption requires an explicit operator action and fresh valid inputs;
reconnection or sensor recovery alone does not restart it. Emergency stop, disarm,
backgrounding, authority loss and the rover watchdog retain their existing
precedence. A navigation source or AR-frame reset invalidates the mission rather
than reinterpreting a previous local planning target.

An observed blocked route may remain held under the existing obstacle policy.
The UI must distinguish `GPS stale`, `heading uncertain`, `tracking lost`,
`obstacle map stale`, `route blocked`, `wheel feedback unavailable` and `paused`.
No GPS loss changes the source to local navigation automatically.

## Components to change

### Terra Rust

Add an outdoor mission/guidance module to `terra-mobile`. It validates timestamped
GPS/heading observations, owns immutable geographic goals and GPS arrival state,
and provides bounded local guidance to the existing arbiter. Extend the arbiter's
guidance interface so a local target can change without replacing the mission
or producing a false arrival acknowledgement. Add source selection and capability
status to the navigation contract. Keep outdoor source health separate from VIO
health; do not inject GPS fixes as pretend VIO observations.

### Terra iOS and bindings

Feed measured Core Location samples through the new typed ingestion API. Preserve
source times and mount-aware heading conversion. Continue existing ARKit, LiDAR,
ground-plane and occupancy acquisition. Expose source, accuracy, readiness and
hold reasons on the phone. Regenerate the UniFFI bindings and framework; update
other generated platform bindings affected by the exported contract. Platforms
without implemented GPS ingestion do not advertise the outdoor capability.

### ARGOS backend and web

Extend Rust-owned API types with versioned source/capability/mission telemetry.
Gate geographic waypoint admission on the acknowledged outdoor capability and
fresh requirements, then send the geographic destination unchanged to Terra.
Preserve control claims, run/revision binding, explicit confirmation and audit
semantics. Recheck readiness until actual publication.

On the GPS map, show the true geographic destination and GPS remaining distance.
Map clicks draft a geographic waypoint only for an outdoor-capable selected rover;
confirmation remains separate. Label the reviewed arrival radius. Local map
overlays remain in the AR frame; they do not claim a geographic obstacle location.
Keep raw GPS visible when navigation is held or unavailable.

## Verification and rollout

Automated regression coverage must establish:

- Correct distance, bearing and body/local steering for north, east, south, west,
  wraparound, calibrated mount rotation and an arbitrary AR yaw/origin.
- No stored geographic-to-AR transform or alignment timer; GPS jumps never move
  the local pose/map. Non-finite, stale, future and replayed samples fail closed.
- A stale GPS/heading, map, pose or missing wheel feedback cannot produce motion.
  Sensor recovery does not resume a paused mission without operator action.
- Intermediate guidance completion cannot complete the GPS mission; three
  distinct valid fixes are required for arrival. Poor accuracy cannot enlarge the
  reviewed radius or report arrival.
- Local L3 navigation still works. In outdoor mode an occupied/unknown path
  prevents driving, and an observed alternative can be planned around an obstacle.
- Unsupported phones reject outdoor selection and geographic commands. Viewer
  accounts, unarmed vehicles and stale run/revision requests remain rejected.
- Browser map clicks only draft; explicit confirmation sends one geographic goal.
  GPS observations remain available during navigation holds.

Run relevant Rust tests, complete Swift regression tests, an iOS build, generated
binding/API drift checks, frontend tests and browser integration tests. Validate
the complete sensor/control path in simulation before physical operation. Deploy
the backend/web capability gate and install the updated phone runtime together;
older phones stay local-only. Physical outdoor arrival, compass bias, braking and
obstacle trials remain separate evidence and are not claimed by a build.

## Scope

This version handles one reviewed outdoor waypoint at a time. It does not add a
road routing service, route queue, RTK receiver, automatic arming, global obstacle
map, automatic navigation-source switching or GPS-only fallback without local
obstacle avoidance.
