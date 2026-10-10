# ARGOS dashboard source

Both the macOS and iOS targets compile these shared sources.

- `ARGOSApp.swift`: app entry point and shared model ownership.
- `Views/`: SwiftUI screens, reusable UI components, and the platform-specific SceneKit view wrapper. Each view has its own file.
- `Controllers/`: fleet observation, backend access, manual driving, and SceneKit scene management.
- `Models/`: telemetry and authority data, occupancy/cloud frames, mission/search state, and drive values.
- `Support/`: map projections, camera geometry, marker placement, draft validation, and shared dashboard styling.

Start with `Views/FleetView.swift` for the main dashboard, `Views/RoverDetailView.swift` for a selected rover, or `Controllers/FleetModel.swift` for fleet state and actions. `Views/RoverJoystick.swift` renders the joystick; `Controllers/RoverDriveController.swift` owns its driving lifecycle.

The Xcode project includes each source file in both app targets. When adding files, update those target memberships; `scripts/create-apple-project.rb` discovers Swift files in these folders when regenerating the project. Generated Rust bindings and frameworks live outside this folder in `../Generated/`.
