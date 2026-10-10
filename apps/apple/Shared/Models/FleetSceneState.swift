import Foundation

/// Platform-neutral operator interaction state, suitable for SwiftUI, RealityKit or Unity.
struct FleetSceneState: Codable, Equatable {
  var frame = "local"
  var selectedID: UInt64?
  var inspecting = false
  var draft: ScenePoint?
  mutating func select(_ id: UInt64?) { if selectedID != id { draft = nil; inspecting = false }; selectedID = id }
  mutating func reconcile(frame: String, ids: Set<UInt64>) {
    if self.frame != frame || selectedID.map({ !ids.contains($0) }) == true { select(nil); draft = nil }
    self.frame = frame
  }
}
struct ScenePoint: Codable, Equatable { let x: Double; let y: Double }
/// Target coordinates are goals, not a reported planned trajectory.
struct FleetSceneVehicle: Codable {
  let id: UInt64
  let membership: String
  let position: ScenePoint?
  let heading: Double?
  let stale: Bool
  let attention: String?
  let goal: ScenePoint?
}
