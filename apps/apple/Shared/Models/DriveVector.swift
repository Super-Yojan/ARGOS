import GameController
import SwiftUI

#if os(macOS)
  import AppKit
#endif

struct DriveVector: Equatable {
  var linear: Double
  var angular: Double
  static let zero = DriveVector(linear: 0, angular: 0)
  static func axes(forward: Double, turn: Double) -> Self {
    func axis(_ v: Double) -> Double { v.isFinite && abs(v) > 0.12 ? max(-1, min(1, v)) : 0 }
    return Self(linear: axis(forward) * 0.5, angular: axis(turn))
  }
  static func keyboard(_ keys: Set<String>) -> Self {
    axes(
      forward: (keys.contains("w") || keys.contains("up") ? 1.0 : 0)
        - (keys.contains("s") || keys.contains("down") ? 1.0 : 0),
      turn: (keys.contains("a") || keys.contains("left") ? 1.0 : 0)
        - (keys.contains("d") || keys.contains("right") ? 1.0 : 0))
  }
}
