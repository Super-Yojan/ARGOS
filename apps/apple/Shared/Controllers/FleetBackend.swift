import Foundation
import MapKit
import SwiftUI

protocol FleetBackend: Sendable {
  func connect(endpoint: String, prefix: String) async throws
  func disconnect() async
  func operatorTelemetry() async -> String
  func operatorCommand(id: UInt64, kind: String, payload: String) async throws
  func localization() async -> String
  func snapshot() async -> FleetSnapshot
  func send(id: UInt64, waypoint: Waypoint) async throws -> String
  func occupancyState() async -> String
  func operatorState() async -> String
  func action(id: UInt64, kind: String, payload: String) async throws
  func fleetAction(kind: String, payload: String) async throws -> String
  func input(id: UInt64, key: String, down: Bool, sequence: UInt64, runId: String, revision: UInt64)
    async throws
  func clearInput(sequence: UInt64) async
  func exportLog(path: String) async throws
  func cancel(id: UInt64) async throws
}
extension FleetBackend {
  func occupancyState() async -> String { "{}" }
  func operatorState() async -> String { "{}" }
  func action(id: UInt64, kind: String, payload: String) async throws {}
  func fleetAction(kind: String, payload: String) async throws -> String { "[]" }
  func input(id: UInt64, key: String, down: Bool, sequence: UInt64, runId: String, revision: UInt64)
    async throws
  {}
  func clearInput(sequence: UInt64) async {}
  func exportLog(path: String) async throws {}

  func localization() async -> String { "{}" }
  func operatorTelemetry() async -> String { "{}" }
  func operatorCommand(id: UInt64, kind: String, payload: String) async throws {
    throw DraftError.invalid
  }
}
