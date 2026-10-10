import GameController
import SwiftUI

#if os(macOS)
  import AppKit
#endif

struct VehicleAuthority: Codable {
  var runID: String
  var requestedLevel: String
  var effectiveLevel: String?
  var activeSource: String
  var safety: String
  var reason: String
  var revision: UInt64
  var token: String?
  var result: String?
  var supportedLevels: [String]
  var age: Double
  enum CodingKeys: String, CodingKey {
    case runID = "run_id"
    case requestedLevel = "requested_level"
    case effectiveLevel = "effective_level"
    case activeSource = "active_source"
    case safety, reason, revision, token, result
    case supportedLevels = "supported_levels"
    case age
  }
  var fresh: Bool { !runID.isEmpty && age.isFinite && age >= 0 && age < 0.5 }
  func acceptsDrive(token: String) -> Bool {
    fresh && self.token == token && result == "accepted"
      && ["teleop", "assisted_teleop"].contains(requestedLevel)
      && effectiveLevel == requestedLevel && ["clear", "active"].contains(safety)
  }
}
