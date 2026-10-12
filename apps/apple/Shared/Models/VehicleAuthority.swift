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
  var globalRoute: [[Double]]?
  var plannerTrace: PlannerTrace?
  enum CodingKeys: String, CodingKey {
    case runID = "run_id"
    case requestedLevel = "requested_level"
    case effectiveLevel = "effective_level"
    case activeSource = "active_source"
    case safety, reason, revision, token, result
    case supportedLevels = "supported_levels"
    case age
    case globalRoute = "global_route"
    case plannerTrace = "planner_trace"
  }
  var fresh: Bool { !runID.isEmpty && age.isFinite && age >= 0 && age < 0.5 }
  func acceptsDrive(token: String) -> Bool {
    fresh && self.token == token && result == "accepted"
      && ["teleop", "assisted_teleop"].contains(requestedLevel)
      && effectiveLevel == requestedLevel && ["clear", "active"].contains(safety)
  }
}

/// Geometry evaluated onboard, in millimetre offsets from the recorded pose.
/// The dashboard projects these points; it never recomputes planner decisions.
struct PlannerTrace: Codable {
  struct Origin: Codable { var x: Double; var y: Double; var yaw: Double }
  struct Velocity: Codable { var linear: Double; var angular: Double }
  struct Candidate: Codable {
    var velocity: Velocity
    var points_mm: [[Int]]
    var rejection: String?
  }
  var age: Double
  var origin: Origin
  var candidates: [Candidate]
  var selected: Int?
  var valid: Bool {
    age.isFinite && age >= 0 && age < 0.5
      && origin.x.isFinite && origin.y.isFinite && origin.yaw.isFinite
      && candidates.count <= 121
      && selected.map { candidates.indices.contains($0) && candidates[$0].rejection == nil } != false
      && candidates.allSatisfy {
        $0.velocity.linear.isFinite && $0.velocity.angular.isFinite
          && !$0.points_mm.isEmpty && $0.points_mm.count <= 16
          && $0.points_mm.allSatisfy { $0.count == 2 && $0.allSatisfy { abs(Double($0)) <= Double(Int32.max) } }
      }
  }
  var accepted: Int { candidates.filter { $0.rejection == nil }.count }
  var rejectionSummary: String {
    Dictionary(grouping: candidates.compactMap(\.rejection), by: { $0 })
      .sorted { $0.key < $1.key }
      .map { "\($0.value.count) \($0.key.replacingOccurrences(of: "_", with: " "))" }
      .joined(separator: " · ")
  }
}
