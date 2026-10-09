# ARGOS scene dashboard

The visual source of truth is [the operator dashboard design](https://super-yojan.dev/ARGOS/design/operator-dashboard/), including its main-layout and third-person wireframes. One place fills the screen. There is no roster, alert queue, navigation menu, metric-card overview, or unit switcher.

The prototype uses a shared local ground plane. Pan and zoom it; select a rover body or its nearby state callout to move the camera closer. 2D and 3D are orthographic camera views of the same scene. Wider returns to the place. Pose, current waypoint activity, missing/stale reports, and transport notices sit beside the selected rover. Off-screen and unknown-position markers remain selectable at the scene boundary without assigning invented positions.

A ground tap in the selected rover view creates a local waypoint draft. Confirm publishes it; Discard removes it. Selection changes clear drafts. Cancel goal remains available while acknowledgement is pending, when transport readiness permits. This is waypoint cancellation, not a physical stop. Urgency rings reflect actual transport/telemetry attention; the prototype does not fabricate a NOW classification or a model thinking trace.

## Real rover asset

`apps/apple/Resources/Rover.scn` is derived from the operator-provided `Downloads/Robot.obj` and `Robot.mtl`. The original CAD mesh contained 3,516,514 triangles. A Blender decimation copy retains 100,000 triangles and its material groups, then ModelIO/SceneKit converts it to a 5.3 MB native resource. Import-only emissive and normal defaults are removed so the chassis and tire colors render correctly. Originals are unchanged.

The asset is grounded and centered, with a normalized two-metre display width. This is a display model, not a calibrated physical footprint. Wide-view minimum sizing keeps bodies selectable. Telemetry positions and waypoint coordinates remain in actual reported local metres. SceneKit shares geometry among rover instances, uses the reported heading, and dims stale bodies. The SwiftUI ground and the SceneKit camera use the same orthographic scale and pitch.

## Current transport boundaries

Cloud/occupancy geometry, onboard VLA traces, voice interpretation, staff allocation, takeover, physical fleet stop, and experiment recording are not exposed by the current checkout's backend. These remain explicitly unavailable. The corner stop is disabled and labelled unavailable; session recording is not claimed. No mock telemetry or terrain is supplied to the shipping UI.

The reference's future point-cloud third-person view is represented by an outside camera around the real rover model and an explicit point-cloud-unavailable note. Existing connection settings remain a single utility action, including phone/simulator profiles. The native apps remain SwiftUI with the Rust/Zenoh transport.


## Automatic positioning quality

The phone publishes `prefix/<rover_id>/localization` once per second (version 1).
ARGOS uses geographic positioning only when every located, online rover has a fresh pose and a fresh usable alignment. Otherwise the shared scene stays local. Changing frame clears the camera focus and unconfirmed waypoint; it does not alter the rover's ARKit or motion-control coordinates.

Terra requests location access while in use. Geographic alignment requires eight continuous seconds of normal, fresh AR tracking, a GPS fix no older than three seconds with horizontal accuracy at most 10 m, and true heading no older than two seconds with accuracy at most 15 degrees. The portrait phone-top direction must project onto the ground. Geographic origin and rotation are captured together and held for that AR session. Interrupted/reset tracking invalidates alignment. Any missing/poor signal immediately reports local positioning; recovery waits eight seconds again. GPS and compass accuracy remain approximate, so this is not indoor/outdoor classification or precision global navigation.

ARGOS discards localization after 2.5 seconds without a report and rejects unsupported/oversized data. Labels are **Local positioning** and **Geographic positioning**, with the reported quality reason near the selected robot. In geographic 2D, the background uses MapKit; 3D remains a flat north/west plane with the real rover mesh, without terrain reconstruction. Ground drafts are converted back into the selected rover's local frame before publication.

Validation: phone hysteresis/heading/reset executable tests; native dashboard transform round-trip/stale/invalid-reference tests; real loopback Zenoh receipt, expiration and disconnect checks; macOS dashboard tests and arm64 iOS builds. Outdoor GPS, compass mounting and geographic overlay alignment still require a real-phone field check. The updated phone build must be installed and granted location access before the existing running phone can publish this new signal.


## Selected rover: cloud, occupancy and driving

Selecting a rover opens its live 3D point cloud and one control panel. TerraPhone sends `pointcloud` at about 3 Hz: LiDAR scene depth when available, otherwise explicitly labeled sparse ARKit feature points. Each frame contains bounded world-space XYZ points, a sequence and AR frame ID; ARGOS keeps up to eight frames and removes the cloud after stale/mismatched metadata or a selection/frame change. This is a short rolling visualization, not a persistent reconstruction. Depth and features use the phone's existing local map ground-height estimate; physical mounting and ground calibration still matter.

Terra already publishes observed `map/occupancy` snapshots at up to 5 Hz. ARGOS consumes their run ID, grid origin, resolution and row-major probability cells. Fresh maps are overlaid under the selected rover: pale free, dark occupied, lightly outlined unknown. Stale snapshots disappear after 0.5 seconds. Waypoint drafts report target-cell knowledge; cells at 65% or higher occupancy block confirmation. Unknown/outside-map targets are visibly marked as unobserved. Geographic scenes rotate/project both map cells and cloud through the same separate alignment used for the rover; outgoing waypoints remain local.

The panel uses the requested L0–L5 vocabulary. L0 maps to `teleop`, L1 to `assisted_teleop`, and L3 to obstacle-aware `waypoint`. L2 (no obstacle avoidance), L4 (target search) and L5 (fleet decision making) remain visible and disabled. Terra's existing `supervised` mode is frontier exploration, not full L4 target search, so ARGOS does not relabel it as L4. Enabled choices also require the rover to advertise support and fresh authority/pose telemetry. Requests carry tokens; the UI reports actual returned authority rather than claiming immediate acceptance.

Manual driving explicitly requests and waits for L0/L1 takeover acknowledgement. Input choices share one bounded stream at 10 Hz, maximum 0.5 m/s and 1 rad/s:
- On-screen joystick: hold/drag; release neutralizes.
- Keyboard: hold WASD or arrow keys after enabling Drive.
- Gamepad: hold the left shoulder and use the left thumbstick; Apple extended-gamepad controllers are supported.

Joystick input has priority while dragged, then held keyboard input, then held gamepad input. Without held input the stream is neutral. Every command includes the current run, authority revision, operator session and increasing sequence. The native sender rechecks authority for moving commands; Terra retains its 0.5-second operator-intent timeout, sensor checks and hardware arming. Release sends neutral; selection, tracking-frame changes, connection loss, changed authority, settings, backgrounding, window focus loss and explicit Stop driving end the drive session. Late takeover replies cannot revive an old selected rover. Phone controls retain priority while actively held; idle phone controls no longer overwrite dashboard commands every tick.

Validation: 17 native dashboard tests (including held/released driving, telemetry loss, delayed takeover and local-frame reset), native Zenoh command receipt/bounds/authority/expiry checks, Terra mobile tests, both native iOS builds, and isolated visual telemetry with joystick release and L1 acknowledgement. Real gamepad hardware and physical rover motion were not exercised. Install the updated TerraPhone build to receive point-cloud telemetry; its existing occupancy snapshots are already published during LiDAR mapping.


Rover scale correction: the CAD model now uses a fixed provisional maximum dimension of 0.85 m (33.5 in), respecting the specified under-35-inch length and width. Geometry no longer grows as the camera zooms out; robot callouts remain selectable. Exact length and width can replace this bound when measured.
