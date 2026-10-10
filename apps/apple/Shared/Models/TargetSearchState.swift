import GameController
import SwiftUI

#if os(macOS)
  import AppKit
#endif

struct TargetSearchState: Codable {
  var runID: String
  var searchID: String
  var phase: String
  var reason: String
  var elapsed: Double
  var detectorAvailable: Bool
  var token: String?
  var result: String?
  var report: TargetSearchReport?
  var age: Double
  enum CodingKeys: String, CodingKey {
    case runID = "run_id"
    case searchID = "search_id"
    case phase, reason
    case elapsed = "elapsed_s"
    case detectorAvailable = "detector_available"
    case token, result, report, age
  }
  var fresh: Bool { age.isFinite && age >= 0 && age < 0.5 }
  var terminal: Bool { ["completed", "cancelled", "timed_out", "exhausted"].contains(phase) }
}
