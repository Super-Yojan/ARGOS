import GameController
import SwiftUI

#if os(macOS)
  import AppKit
#endif

struct HardwareState: Codable {
  var simulated: Bool? = nil
  var ready: Bool
  var armed: Bool
  var arming: Bool
  var reason: String
  var token: String?
  var result: String?
  var age: Double
  var fresh: Bool { age.isFinite && age >= 0 && age < 0.5 }
}
