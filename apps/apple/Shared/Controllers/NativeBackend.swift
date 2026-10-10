import Foundation
import MapKit
import SwiftUI

actor NativeBackend: FleetBackend {
  private let client = ArgosClient()
  func occupancyState() async -> String { client.occupancySnapshot() }
  func operatorState() async -> String { client.operatorSnapshot() }
  func action(id: UInt64, kind: String, payload: String) async throws {
    _ = try client.operatorAction(roverId: id, kind: kind, payload: payload)
  }
  func fleetAction(kind: String, payload: String) async throws -> String {
    try client.fleetAction(kind: kind, payload: payload)
  }
  func input(id: UInt64, key: String, down: Bool, sequence: UInt64, runId: String, revision: UInt64)
    async throws
  {
    try client.inputEvent(
      roverId: id, key: key, down: down, sequence: sequence, runId: runId, revision: revision)
  }
  func clearInput(sequence: UInt64) async { client.clearInputEvent(sequence: sequence) }
  func exportLog(path: String) async throws {
    let client = self.client
    try await Task.detached { try client.exportSession(destination: path) }.value
  }

  func connect(endpoint: String, prefix: String) async throws {
    try client.connect(config: ConnectionConfig(endpoint: endpoint, prefix: prefix))
  }
  func disconnect() async { client.disconnect() }
  func operatorTelemetry() async -> String { client.operatorSnapshot() }
  func operatorCommand(id: UInt64, kind: String, payload: String) async throws {
    try client.operatorCommand(roverId: id, kind: kind, payload: payload)
  }
  func localization() async -> String { client.localizationSnapshot() }
  func snapshot() async -> FleetSnapshot { client.snapshot() }
  func send(id: UInt64, waypoint: Waypoint) async throws -> String {
    try client.sendGoal(roverId: id, waypoint: waypoint)
  }
  func cancel(id: UInt64) async throws { try client.cancelGoal(roverId: id) }
}
