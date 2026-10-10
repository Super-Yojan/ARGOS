import Foundation
import MapKit
import SwiftUI

@MainActor final class FleetModel: ObservableObject {
  @Published private(set) var snapshot = FleetSnapshot(
    link: "disconnected", fleetAge: nil, rovers: [], notice: nil)
  @Published private(set) var localizations: [UInt64: GeographicReference] = [:]
  @Published private(set) var geographicOrigin: CLLocationCoordinate2D?
  @Published private(set) var frameKey = "local"
  @Published private(set) var sceneRovers: [RoverView] = []
  @Published private(set) var hardware: [UInt64: HardwareState] = [:]
  @Published private(set) var hardwarePending: [UInt64: Date] = [:]
  @Published private(set) var searchReports: [UInt64: [TargetSearchReport]] = [:]
  private var reportAckTimes: [String: Date] = [:]
  @Published private(set) var searches: [UInt64: TargetSearchState] = [:]
  @Published private(set) var searchPending: [UInt64: (token: String, started: Date)] = [:]
  @Published private(set) var authorities: [UInt64: VehicleAuthority] = [:]
  @Published private(set) var occupancyMaps: [UInt64: OccupancyFrame] = [:]
  @Published private(set) var clouds: [UInt64: PointCloudFrame] = [:]
  @Published private(set) var autonomyPending: [UInt64: String] = [:]
  @Published private(set) var busy = false
  @Published var error: String?
  @Published var selected: UInt64? { didSet { if oldValue != selected { clearInput(); cancelPendingWaypoint() } } }
  @Published private(set) var operatorStates: [String: OperatorState] = [:]
  @Published private(set) var occupancyStates: [String: OccupancyState] = [:]
  @Published private(set) var changingAuthority: Set<UInt64> = []
  @Published var fleetResult: String?
  @Published var exportedLog: URL?
  @Published private(set) var sessionEpoch: UInt64 = 0
  @Published private(set) var pendingWaypointID: UInt64?
  private var waypointOperation: UInt64 = 0
  private var inputSequence: UInt64 = 0
  private let backend: any FleetBackend
  private var observation: Task<Void, Never>?
  var observing: Bool { observation != nil }
  init(backend: any FleetBackend = NativeBackend()) { self.backend = backend }
  func connect(endpoint: String, prefix: String) async {
    guard !busy else { return }
    busy = true
    cancelPendingWaypoint()
    sessionEpoch &+= 1
    let epoch = sessionEpoch
    error = nil
    stopObservation()
    clearInput()
    await backend.disconnect()
    clearSessionState()
    defer { busy = false }
    do {
      try await backend.connect(endpoint: endpoint, prefix: prefix)
      guard epoch == sessionEpoch else { return }
      await refresh()
      startObservation()
    } catch {
      guard epoch == sessionEpoch else { return }
      self.error = error.localizedDescription
      await refresh()
    }
  }
  func disconnect() async {
    cancelPendingWaypoint()
    sessionEpoch &+= 1
    stopObservation()
    clearInput()
    await backend.disconnect()
    clearSessionState()
    await refresh()
  }
  func sceneDocument() -> String {
    struct Document: Encodable { let version = 1; let frame: String; let vehicles: [FleetSceneVehicle] }
    let vehicles = sceneRovers.map { rover in
      FleetSceneVehicle(id: rover.id, membership: rover.membership,
        position: rover.pose.map { ScenePoint(x: $0.x, y: $0.y) }, heading: rover.pose?.yaw,
        stale: rover.poseAge.map { $0 >= SupervisionTiming.staleAfter } ?? true,
        attention: DashboardPresentation.attentionReason(rover),
        goal: rover.goal.map { ScenePoint(x: $0.x, y: $0.y) })
    }
    return (try? JSONEncoder().encode(Document(frame: frameKey, vehicles: vehicles))).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
  }
  private func clearSessionState() {
    selected = nil
    snapshot = FleetSnapshot(link: "disconnected", fleetAge: nil, rovers: [], notice: nil)
    localizations = [:]; geographicOrigin = nil; frameKey = "local"
    sceneRovers = []; hardware = [:]; hardwarePending = [:]; searchReports = [:]
    reportAckTimes = [:]; searches = [:]; searchPending = [:]; authorities = [:]
    occupancyMaps = [:]; clouds = [:]; autonomyPending = [:]
    operatorStates = [:]; occupancyStates = [:]; changingAuthority = []
    fleetResult = nil; exportedLog = nil
  }
  func refresh() async {
    let epoch = sessionEpoch
    let nextSnapshot = await backend.snapshot()
    guard epoch == sessionEpoch, !Task.isCancelled else { return }
    snapshot = nextSnapshot
    let operatorStatePayload = await backend.operatorState()
    guard epoch == sessionEpoch, !Task.isCancelled else { return }
    let data = Data(operatorStatePayload.utf8)
    let occupancyPayload = await backend.occupancyState()
    guard epoch == sessionEpoch, !Task.isCancelled else { return }
    occupancyStates =
      (try? JSONDecoder().decode(
        [String: OccupancyState].self, from: Data(occupancyPayload.utf8))) ?? [:]
    let previous = operatorStates
    operatorStates = (try? JSONDecoder().decode([String: OperatorState].self, from: data)) ?? [:]
    if let selected,
      previous[String(selected)]?.status?.revision
        != operatorStates[String(selected)]?.status?.revision
        || previous[String(selected)]?.status?.run_id
          != operatorStates[String(selected)]?.status?.run_id
    {
      clearInput()
    }

    let operatorPayload = await backend.operatorTelemetry()
    guard epoch == sessionEpoch, !Task.isCancelled else { return }
    let operatorData =
      (try? JSONDecoder().decode([String: OperatorTelemetry].self, from: Data(operatorPayload.utf8)))
      ?? [:]
    hardware = Dictionary(
      uniqueKeysWithValues: operatorData.compactMap { key, value in
        guard let id = UInt64(key), let state = value.hardware, SupervisionTiming.usable(state.age) else { return nil }
        return (id, state)
      })
    for (id, started) in hardwarePending {
      if hardware[id]?.armed == true || hardware[id]?.arming == true
        || hardware[id]?.result?.hasPrefix("rejected") == true
      {
        hardwarePending.removeValue(forKey: id)
      } else if Date().timeIntervalSince(started) >= 4 {
        hardwarePending.removeValue(forKey: id)
        error = "Arm not confirmed · \(hardware[id]?.reason ?? "Hardware status unavailable")"
      }
    }
    authorities = Dictionary(
      uniqueKeysWithValues: operatorData.compactMap { key, value in
        guard let id = UInt64(key), let authority = value.authority else { return nil }
        return (id, authority)
      })
    searches = Dictionary(
      uniqueKeysWithValues: operatorData.compactMap { key, value in
        guard let id = UInt64(key), let search = value.search,
          search.runID == authorities[id]?.runID
        else { return nil }
        return (id, search)
      })
    for (key, data) in operatorData {
      guard let id = UInt64(key) else { continue }
      for report in data.reports ?? [] {
        let identity = "\(id):\(report.runID):\(report.searchID):\(report.reportID)"
        if !(searchReports[id] ?? []).contains(where: {
          $0.runID == report.runID && $0.searchID == report.searchID
            && $0.reportID == report.reportID
        }) {
          var history = searchReports[id] ?? []
          history.append(report)
          searchReports[id] = Array(history.suffix(128))
        }
        if reportAckTimes[identity].map({ Date().timeIntervalSince($0) < 1 }) == true { continue }
        let ack: [String: Any] = [
          "version": 1, "run_id": report.runID, "search_id": report.searchID,
          "report_id": report.reportID, "token": UUID().uuidString,
        ]
        if let payload = try? JSONSerialization.data(withJSONObject: ack) {
          do {
            guard epoch == sessionEpoch, !Task.isCancelled else { return }
            try await backend.operatorCommand(
              id: id, kind: "search/report/ack", payload: String(decoding: payload, as: UTF8.self))
            guard epoch == sessionEpoch, !Task.isCancelled else { return }
            reportAckTimes[identity] = Date()
          } catch {
            guard epoch == sessionEpoch, !Task.isCancelled else { return }
            self.error = "Report acknowledgement failed · \(error.localizedDescription)"
          }
        }
      }
    }
    reportAckTimes = reportAckTimes.filter { Date().timeIntervalSince($0.value) < 5 }
    for (id, pending) in searchPending {
      if authorities[id]?.token == pending.token {
        searchPending.removeValue(forKey: id)
        if authorities[id]?.result == "rejected" {
          error = "Search request rejected · \(authorities[id]?.reason ?? "unknown")"
        }
      } else if Date().timeIntervalSince(pending.started) > 4 {
        searchPending.removeValue(forKey: id)
        error = "Search request unconfirmed · check rover status"
      }
    }
    clouds = Dictionary(
      uniqueKeysWithValues: operatorData.compactMap { key, value in
        guard let id = UInt64(key), let cloud = value.cloud, cloud.usable else { return nil }
        return (id, cloud)
      })
    occupancyMaps = Dictionary(
      uniqueKeysWithValues: operatorData.compactMap { key, value in
        guard let id = UInt64(key), let map = value.occupancy, map.usable, map.roverID == id,
          map.runID == authorities[id]?.runID
        else { return nil }
        return (id, map)
      })
    for (id, token) in autonomyPending where authorities[id]?.token == token {
      autonomyPending.removeValue(forKey: id)
    }
    let payload = await backend.localization()
    guard epoch == sessionEpoch, !Task.isCancelled else { return }
    let decoded =
      (try? JSONDecoder().decode([String: GeographicReference].self, from: Data(payload.utf8)))
      ?? [:]
    localizations = Dictionary(
      uniqueKeysWithValues: decoded.compactMap { key, value in UInt64(key).map { ($0, value) } })
    let located = snapshot.rovers.filter { $0.pose != nil }
    let geographic =
      !located.isEmpty
      && located.allSatisfy {
        $0.membership == "online" && ($0.poseAge ?? .infinity) < SupervisionTiming.staleAfter
          && localizations[$0.id]?.usable == true
      }
    let nextKey =
      geographic
      ? located.sorted { $0.id < $1.id }.map { "\($0.id):\(localizations[$0.id]!.frameID)" }.joined(
        separator: ",")
      : "local:"
        + snapshot.rovers.sorted { $0.id < $1.id }.map {
          "\($0.id):\(localizations[$0.id]?.frameID ?? "unknown")"
        }.joined(separator: ",")
    if frameKey != nextKey {
      frameKey = nextKey
      geographicOrigin =
        geographic ? localizations[located.sorted { $0.id < $1.id }[0].id]?.origin : nil
    }
    sceneRovers = snapshot.rovers.map { rover in
      guard let origin = geographicOrigin, let ref = localizations[rover.id], ref.usable else {
        return rover
      }
      var result = rover
      if let pose = rover.pose {
        let point = ref.scene(x: pose.x, y: pose.y, origin: origin)
        result.pose = RoverPose(x: point.x, y: point.y, yaw: pose.yaw + ref.rotation!)
      }
      if var goal = rover.goal {
        let point = ref.scene(x: goal.x, y: goal.y, origin: origin)
        goal.x = point.x
        goal.y = point.y
        result.goal = goal
      }
      return result
    }
    if let selected, !snapshot.rovers.contains(where: { $0.id == selected }) { self.selected = nil }
    if selected == nil {
      selected =
        snapshot.rovers.first(where: { $0.membership == "online" })?.id ?? snapshot.rovers.first?.id
    }
  }
  func motorsArmed(_ id: UInt64) -> Bool {
    hardware[id]?.fresh == true && hardware[id]?.armed == true
  }
  func requestHardware(id: UInt64, arm: Bool) async {
    guard !arm || (driveAvailable(id) && hardware[id]?.ready == true) else {
      error = "Fresh, ready rover required to arm"
      return
    }
    do {
      var payload: [String: Any] = ["action": arm ? "arm" : "disarm", "token": UUID().uuidString]
      if arm {
        guard let authority = authorities[id] else {
          error = "Vehicle authority unavailable"
          return
        }
        payload["run_id"] = authority.runID
        payload["authority_revision"] = authority.revision
      }
      try await backend.operatorCommand(
        id: id, kind: "hardware",
        payload: String(decoding: JSONSerialization.data(withJSONObject: payload), as: UTF8.self))
      if arm { hardwarePending[id] = Date() } else { hardwarePending.removeValue(forKey: id) }
      error = nil
    } catch { self.error = error.localizedDescription }
  }
  func driveAvailable(_ id: UInt64) -> Bool {
    snapshot.link == "healthy" && authorities[id]?.fresh == true
      && snapshot.rovers.contains {
        $0.id == id && $0.membership == "online" && ($0.poseAge ?? .infinity) < 0.5
      }
  }
  func waypointAvailable(_ id: UInt64) -> Bool {
    guard snapshot.link == "healthy", let authority = authorities[id],
      SupervisionTiming.usable(authority.age), let hardware = hardware[id],
      SupervisionTiming.usable(hardware.age), hardware.ready, hardware.armed else { return false }
    return snapshot.rovers.contains { $0.id == id && $0.membership == "online" && $0.poseAge.map(SupervisionTiming.usable) == true }
  }
  func selectAutonomy(id: UInt64, level: String) async -> String? {
    guard (level == "waypoint" ? waypointAvailable(id) : driveAvailable(id)), authorities[id]?.supportedLevels.contains(level) == true else {
      error = "Fresh vehicle authority and supported level required"
      return nil
    }
    let token = UUID().uuidString
    do {
      let data = try JSONSerialization.data(withJSONObject: ["level": level, "token": token])
      try await backend.operatorCommand(
        id: id, kind: "autonomy", payload: String(decoding: data, as: UTF8.self))
      autonomyPending[id] = token
      return token
    } catch {
      self.error = error.localizedDescription
      return nil
    }
  }
  func startSearch(
    id: UInt64, targetClass: String, minX: Double, minY: Double, maxX: Double, maxY: Double,
    budget: Double
  ) async {
    guard driveAvailable(id), let authority = authorities[id],
      authority.requestedLevel == "target_search",
      authority.supportedLevels.contains("target_search")
    else {
      error = "Select L4 and wait for rover acknowledgement"
      return
    }
    guard [minX, minY, maxX, maxY, budget].allSatisfy({ $0.isFinite }), maxX > minX, maxY > minY,
      maxX - minX <= 100, maxY - minY <= 100, budget > 0, budget <= 3600, !targetClass.isEmpty
    else {
      error = "Enter a target, valid local bounds and a budget up to 3600 seconds"
      return
    }
    let token = UUID().uuidString
    let payload: [String: Any] = [
      "version": 1, "run_id": authority.runID, "search_id": UUID().uuidString, "token": token,
      "target_class": targetClass,
      "bounds": ["min_x": minX, "min_y": minY, "max_x": maxX, "max_y": maxY],
      "time_budget_s": budget,
    ]
    await publishSearch(id: id, kind: "search", payload: payload, token: token)
  }
  func searchAction(id: UInt64, action: String) async {
    guard let search = searches[id], search.fresh else {
      error = "Fresh search status required"
      return
    }
    let token = UUID().uuidString
    await publishSearch(
      id: id, kind: "search/action",
      payload: [
        "version": 1, "run_id": search.runID, "search_id": search.searchID, "token": token,
        "action": action,
      ], token: token)
  }
  private func publishSearch(id: UInt64, kind: String, payload: [String: Any], token: String) async
  {
    do {
      try await backend.operatorCommand(
        id: id, kind: kind,
        payload: String(decoding: JSONSerialization.data(withJSONObject: payload), as: UTF8.self))
      searchPending[id] = (token, Date())
      error = nil
    } catch { self.error = error.localizedDescription }
  }
  func drive(
    id: UInt64, vector: DriveVector, authority: VehicleAuthority, session: String, sequence: UInt64
  ) async throws {
    guard vector == .zero || motorsArmed(id) else {
      throw NSError(
        domain: "ARGOS", code: 1,
        userInfo: [NSLocalizedDescriptionKey: "Motors are not confirmed armed"])
    }
    let value: [String: Any] = [
      "linear": vector.linear, "angular": vector.angular, "run_id": authority.runID,
      "authority_revision": authority.revision, "operator_session_id": session,
      "sequence": sequence,
    ]
    let data = try JSONSerialization.data(withJSONObject: value)
    try await backend.operatorCommand(
      id: id, kind: "teleop", payload: String(decoding: data, as: UTF8.self))
  }
  func mapScenePoint(id: UInt64, x: Double, y: Double) -> (x: Double, y: Double)? {
    if let origin = geographicOrigin {
      guard let ref = localizations[id], ref.usable else { return nil }
      return ref.scene(x: x, y: y, origin: origin)
    }
    return (x, y)
  }
  func waypointCell(id: UInt64, x: Double, y: Double) -> Int? {
    guard let map = occupancyMaps[id], map.usable else { return nil }
    var local = (x: x, y: y)
    if let origin = geographicOrigin {
      guard let ref = localizations[id], ref.usable else { return nil }
      local = ref.local(x: x, y: y, origin: origin)
    }
    return map.value(x: local.x, y: local.y)
  }
  func sceneCloud(_ id: UInt64) -> PointCloudFrame? {
    guard var cloud = clouds[id] else { return nil }
    if let ref = localizations[id], cloud.frameID != ref.frameID { return nil }
    if let origin = geographicOrigin {
      guard let ref = localizations[id], ref.usable, cloud.frameID == ref.frameID else {
        return nil
      }
      cloud.points = cloud.points.map { point in
        let transformed = ref.scene(x: point[0], y: point[1], origin: origin)
        return [transformed.x, transformed.y, point[2]]
      }
    }
    cloud.frameID += ":\(id):" + frameKey
    return cloud
  }
  func canPlaceGeographically(_ id: UInt64) -> Bool {
    geographicOrigin == nil || localizations[id]?.usable == true
  }
  func state(_ id: UInt64) -> OperatorState? { operatorStates[String(id)] }
  func occupancy(_ id: UInt64) -> OccupancyState? {
    guard let map = occupancyStates[String(id)], map.grid.rover_id == id, map.grid.valid,
      let run = state(id)?.status?.run_id, run == map.grid.run_id
    else { return nil }
    return map
  }
  func clearInput() {
    inputSequence += 1
    let sequence = inputSequence
    Task { await backend.clearInput(sequence: sequence) }
  }
  func input(id: UInt64, key: String, down: Bool) {
    guard let authority = state(id)?.status, let run = authority.run_id else { return }
    inputSequence += 1
    let sequence = inputSequence
    Task {
      do {
        try await backend.input(
          id: id, key: key, down: down, sequence: sequence, runId: run, revision: authority.revision
        )
      } catch { self.error = error.localizedDescription }
    }
  }
  func action(id: UInt64, kind: String, values: [String: Any]) async {
    if kind == "autonomy" {
      guard !changingAuthority.contains(id) else { return }
      changingAuthority.insert(id)
    }
    defer { if kind == "autonomy" { changingAuthority.remove(id) } }
    do {
      let data = try JSONSerialization.data(withJSONObject: values)
      try await backend.action(id: id, kind: kind, payload: String(decoding: data, as: UTF8.self))
      await refresh()
    } catch { self.error = error.localizedDescription }
  }
  func fleetAction(stop: Bool) async {
    cancelPendingWaypoint()
    clearInput()
    do {
      fleetResult = try await backend.fleetAction(
        kind: stop ? "safety" : "autonomy",
        payload: stop ? "{\"action\":\"stop\"}" : "{\"level\":\"teleop\"}")
      await refresh()
    } catch { self.error = error.localizedDescription }
  }
  func exportLog() async {
    do {
      let url = FileManager.default.temporaryDirectory.appendingPathComponent(
        "ARGOS-session-\(UUID().uuidString).jsonl")
      try await backend.exportLog(path: url.path)
      exportedLog = url
    } catch { self.error = error.localizedDescription }
  }
  func startObservation() {
    guard observation == nil, snapshot.link != "disconnected" else { return }
    observation = Task { [weak self] in
      while !Task.isCancelled {
        guard let self else { return }
        await self.refresh()
        do { try await Task.sleep(for: .milliseconds(200)) } catch { return }
      }
    }
  }
  func stopObservation() {
    observation?.cancel()
    observation = nil
  }
  func cancelPendingWaypoint() {
    waypointOperation &+= 1
    pendingWaypointID = nil
  }
  func send(id: UInt64, waypoint: Waypoint) async -> Bool {
    guard waypointAvailable(id) else {
      error = "Arm the rover before executing a waypoint"
      return false
    }
    guard !busy else { return false }
    busy = true
    waypointOperation &+= 1
    let operation = waypointOperation
    let epoch = sessionEpoch
    pendingWaypointID = id
    error = nil
    defer { busy = false; if operation == waypointOperation { pendingWaypointID = nil } }
    do {
      if authorities[id]?.requestedLevel != "waypoint" {
        guard let token = await selectAutonomy(id: id, level: "waypoint") else { return false }
        let deadline = Date().addingTimeInterval(SupervisionTiming.acknowledgmentTimeout)
        var accepted = false
        while Date() < deadline {
          guard operation == waypointOperation, epoch == sessionEpoch, !Task.isCancelled else { return false }
          await refresh()
          guard operation == waypointOperation, epoch == sessionEpoch, !Task.isCancelled else { return false }
          if let authority = authorities[id], authority.token == token {
            guard authority.result == "accepted", authority.requestedLevel == "waypoint" else {
              error = "Waypoint mode rejected · \(authority.reason)"
              return false
            }
            accepted = true
            break
          }
          try await Task.sleep(for: .milliseconds(100))
        }
        guard accepted else {
          error = "Waypoint mode was not acknowledged"
          return false
        }
      }
      guard operation == waypointOperation, epoch == sessionEpoch, !Task.isCancelled else { return false }
      guard waypointAvailable(id) else {
        error = "Rover lost readiness before waypoint execution"
        return false
      }
      _ = try await backend.send(id: id, waypoint: waypoint)
      await refresh()
      return true
    } catch { if operation == waypointOperation, epoch == sessionEpoch { self.error = error.localizedDescription }; return false }
  }
  func cancel(id: UInt64) async {
    if busy, pendingWaypointID == id {
      cancelPendingWaypoint()
      do { try await backend.cancel(id: id) } catch { self.error = error.localizedDescription }
      return
    }
    guard !busy else { return }
    busy = true
    error = nil
    defer { busy = false }
    do {
      try await backend.cancel(id: id)
      await refresh()
    } catch { self.error = error.localizedDescription }
  }
}
