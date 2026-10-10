import Foundation
import MapKit
import SwiftUI

enum WaypointDraft {
  static func make(first: String, second: String, geographic: Bool) throws -> Waypoint {
    guard let a = Double(first.trimmingCharacters(in: .whitespaces)),
      let b = Double(second.trimmingCharacters(in: .whitespaces)), a.isFinite, b.isFinite
    else { throw DraftError.invalid }
    return geographic
      ? .geographic(latitude: a, longitude: b, yaw: nil) : .local(x: a, y: b, yaw: nil)
  }
}
// Same local tangent plane as terra-waypoint; +x north, +y west.
