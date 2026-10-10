import GameController
import SwiftUI

#if os(macOS)
  import AppKit
#endif

struct AutonomyChoice: Identifiable {
  let id: Int
  let title: String
  let wire: String?
  static let all = [
    Self(id: 0, title: "Fully teleop", wire: "teleop"),
    Self(id: 1, title: "Assisted teleop", wire: "assisted_teleop"),
    Self(id: 2, title: "Waypoint · no avoidance", wire: "waypoint_direct"),
    Self(id: 3, title: "Waypoint · obstacle aware", wire: "waypoint"),
    Self(id: 4, title: "Target search", wire: "target_search"),
    Self(id: 5, title: "Decision making", wire: nil),
  ]
}
