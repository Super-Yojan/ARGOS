import Foundation
import MapKit
import SwiftUI

// Native snapshots share one operator document. Test/alternate backends may still
// supply the legacy and current projections separately.
struct FleetTelemetryPayloads: Sendable {
  let operatorState: String
  let occupancyState: String
  let operatorTelemetry: String
  let localization: String
  let sharedOperatorDocument: Bool
  let capturedAt: ContinuousClock.Instant

  init(operatorState: String, occupancyState: String, operatorTelemetry: String,
    localization: String, sharedOperatorDocument: Bool = false,
    capturedAt: ContinuousClock.Instant = .now)
  {
    self.operatorState = operatorState
    self.occupancyState = occupancyState
    self.operatorTelemetry = operatorTelemetry
    self.localization = localization
    self.sharedOperatorDocument = sharedOperatorDocument
    self.capturedAt = capturedAt
  }

  func decode() -> DecodedFleetTelemetry {
    let decoder = JSONDecoder()
    let states: [String: OperatorState]
    let telemetry: [String: OperatorTelemetry]
    if sharedOperatorDocument {
      let document = try? decoder.decode(OperatorDocument.self, from: Data(operatorState.utf8))
      states = document?.states ?? [:]
      telemetry = document?.telemetry ?? [:]
    } else {
      states = (try? decoder.decode([String: OperatorState].self,
        from: Data(operatorState.utf8))) ?? [:]
      telemetry = (try? decoder.decode([String: OperatorTelemetry].self,
        from: Data(operatorTelemetry.utf8))) ?? [:]
    }
    return DecodedFleetTelemetry(
      operatorStates: states,
      occupancyStates: (try? decoder.decode([String: OccupancyState].self,
        from: Data(occupancyState.utf8))) ?? [:],
      operatorTelemetry: telemetry,
      localizations: (try? decoder.decode([String: GeographicReference].self,
        from: Data(localization.utf8))) ?? [:],
      clouds: Dictionary(uniqueKeysWithValues: telemetry.compactMap { key, value in
        guard let id = UInt64(key), let cloud = value.cloud, cloud.usable else { return nil }
        return (id, cloud)
      }),
      occupancyMaps: Dictionary(uniqueKeysWithValues: telemetry.compactMap { key, value in
        guard let id = UInt64(key), let map = value.occupancy, map.usable, map.roverID == id,
          map.runID == value.authority?.runID
        else { return nil }
        return (id, map)
      }))
  }

  private struct OperatorDocument: Decodable {
    let states: [String: OperatorState]
    let telemetry: [String: OperatorTelemetry]

    init(from decoder: Decoder) throws {
      // Both projections reuse JSONDecoder's parsed document. Keep independent
      // failure behavior: malformed telemetry must not hide valid legacy state.
      states = (try? [String: OperatorState](from: decoder)) ?? [:]
      telemetry = (try? [String: OperatorTelemetry](from: decoder)) ?? [:]
    }
  }
}

struct DecodedFleetTelemetry {
  let operatorStates: [String: OperatorState]
  let occupancyStates: [String: OccupancyState]
  let operatorTelemetry: [String: OperatorTelemetry]
  let localizations: [String: GeographicReference]
  let clouds: [UInt64: PointCloudFrame]
  let occupancyMaps: [UInt64: OccupancyFrame]

  func aged(by elapsed: Double) -> DecodedFleetTelemetry {
    let clouds = clouds.mapValues { value in
      var cloud = value
      cloud.age = advancedTelemetryAge(cloud.age, by: elapsed)
      return cloud
    }.filter { $0.value.fresh }
    let maps = occupancyMaps.mapValues { value in
      var map = value
      map.age = advancedTelemetryAge(map.age, by: elapsed)
      return map
    }.filter { $0.value.age.isFinite && $0.value.age >= 0 && $0.value.age < 0.5 }
    return DecodedFleetTelemetry(
      operatorStates: operatorStates.mapValues { value in
        var state = value
        state.age = state.age.map { advancedTelemetryAge($0, by: elapsed) }
        state.proposal_remaining = state.proposal_remaining.map { max(0, $0 - elapsed) }
        return state
      },
      occupancyStates: occupancyStates.mapValues {
        OccupancyState(grid: $0.grid, age: advancedTelemetryAge($0.age, by: elapsed))
      },
      operatorTelemetry: operatorTelemetry.mapValues { value in
        var data = value
        if var hardware = data.hardware {
          hardware.age = advancedTelemetryAge(hardware.age, by: elapsed)
          data.hardware = hardware
        }
        if var authority = data.authority {
          authority.age = advancedTelemetryAge(authority.age, by: elapsed)
          if var trace = authority.plannerTrace {
            trace.age = advancedTelemetryAge(trace.age, by: elapsed)
            authority.plannerTrace = trace
          }
          data.authority = authority
        }
        if var search = data.search {
          search.age = advancedTelemetryAge(search.age, by: elapsed)
          data.search = search
        }
        if var cloud = data.cloud {
          cloud.age = advancedTelemetryAge(cloud.age, by: elapsed)
          data.cloud = cloud
        }
        if var map = data.occupancy {
          map.age = advancedTelemetryAge(map.age, by: elapsed)
          data.occupancy = map
        }
        return data
      },
      localizations: localizations.mapValues { value in
        var reference = value
        reference.age = advancedTelemetryAge(reference.age, by: elapsed)
        return reference
      },
      clouds: clouds, occupancyMaps: maps)
  }
}

func telemetryElapsed(since instant: ContinuousClock.Instant) -> Double {
  let duration = instant.duration(to: .now).components
  return max(0, Double(duration.seconds) + Double(duration.attoseconds) / 1e18)
}

func advancedTelemetryAge(_ age: Double, by elapsed: Double) -> Double {
  // Invalid ages must stay invalid rather than becoming fresh after a delay.
  age.isFinite && age >= 0 ? age + elapsed : age
}

protocol FleetBackend: Sendable {
  func connect(endpoint: String, prefix: String) async throws
  func disconnect() async
  func telemetryPayloads() async -> FleetTelemetryPayloads
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
  func telemetryPayloads() async -> FleetTelemetryPayloads {
    let capturedAt = ContinuousClock.now
    let state = await operatorState()
    let occupancy = await occupancyState()
    let telemetry = await operatorTelemetry()
    let positioning = await localization()
    return FleetTelemetryPayloads(operatorState: state, occupancyState: occupancy,
      operatorTelemetry: telemetry, localization: positioning, capturedAt: capturedAt)
  }

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
