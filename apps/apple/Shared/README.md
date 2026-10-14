# ARGOS dashboard source

Both the macOS and iOS targets compile these shared sources.

- `ARGOSApp.swift`: app entry point and shared model ownership.
- `Views/`: SwiftUI screens, reusable UI components, and the platform-specific SceneKit view wrapper. Each view has its own file.
- `Controllers/`: fleet observation, backend access, manual driving, and SceneKit scene management.
- `Models/`: telemetry and authority data, occupancy/cloud frames, mission/search state, and drive values.
- `Support/`: map projections, camera geometry, marker placement, draft validation, and shared dashboard styling.

Start with `Views/FleetView.swift` for the main dashboard, `Views/RoverDetailView.swift` for a selected rover, or `Controllers/FleetModel.swift` for fleet state and actions. `Views/RoverJoystick.swift` renders the joystick; `Controllers/RoverDriveController.swift` owns its driving lifecycle.

The Xcode project includes each source file in both app targets. When adding files, update those target memberships; `scripts/create-apple-project.rb` discovers Swift files in these folders when regenerating the project. Generated Rust bindings and frameworks live outside this folder in `../Generated/`.

`Models/FleetSceneState.swift` defines renderer-independent selection/draft state and scene vehicle records. `Controllers/RouterController.swift` owns native router lifecycle and Bonjour discovery. The same Rust router implementation backs the app and standalone `argos-router` executable; deployment details are in `docs/deployment-router.md`.

## Runtime updates

`FleetModel` still refreshes at a bounded cadence so receive ages and safety checks
advance even when telemetry is unchanged. `NativeBackend` reads one operator
document per refresh; both legacy state and current telemetry decode from that
document off the main actor. Grid/cloud content validation also runs there.

Rendering resources follow content identity rather than camera movement:

- Occupancy textures are keyed by rover, run, sequence and grid geometry, with a
  32 MiB least-recently-used cache. Pan/zoom/orbit projects three corners and draws
  one image. Unknown-cell dots scale with the grid; occupancy thresholds stay unchanged.
- Cloud scene coordinates are cached by source sequence and geographic transform.
  Coordinate alignment or session changes reset history, even if a sequence repeats.
  Unchanged clouds retain their SceneKit geometry during camera movement.
- MapKit retains its view and updates a bound camera position.

Publishers must change sequences when grid/cloud content changes. Updated Terra
and Zorvane publishers send unchanged-content heartbeats on
`map/occupancy/heartbeat` and `pointcloud/heartbeat`, referencing the last full
payload's rover/run or frame/sequence identity. ARGOS only renews an existing
matching baseline; heartbeats never create missing content or renew authority.
Publishers only send them when new sensor observations arrive, so a stalled
sensor still becomes stale. Periodic full baselines recover late subscribers.
