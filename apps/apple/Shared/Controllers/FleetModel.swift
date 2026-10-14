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
  private var refreshSequence: UInt64 = 0
  private var appliedRefreshSequence: UInt64 = 0
  private var telemetryAppliedAt: ContinuousClock.Instant?
  private var elapsedSinceTelemetry: Double {
    telemetryAppliedAt.map { telemetryElapsed(since: $0) } ?? .infinity
  }
  private struct CloudPlacement: Equatable {
    let sequence: UInt64
    let sourceFrame: String
    let sceneFrame: String
    // Only coordinate-transform fields belong here, not changing receive ages.
    let transform: [Double]
  }
  private var sceneCloudCache: [UInt64: (placement: CloudPlacement, cloud: PointCloudFrame)] = [:]
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
    sceneCloudCache = [:]
    telemetryAppliedAt = nil
    operatorStates = [:]; occupancyStates = [:]; changingAuthority = []
    fleetResult = nil; exportedLog = nil
  }
  func refresh() async {
    let epoch = sessionEpoch
    refreshSequence &+= 1
    let request = refreshSequence
    let snapshotCapturedAt = ContinuousClock.now
    var nextSnapshot = await backend.snapshot()
    guard epoch == sessionEpoch, !Task.isCancelled else { return }
    let payloads = await backend.telemetryPayloads()
    guard epoch == sessionEpoch, !Task.isCancelled else { return }
    // JSON parsing and large grid/cloud allocations must not block gestures or
    // native controls. Only apply a completed decode to its original session.
    let decoded = await Task.detached(priority: .userInitiated) {
      payloads.decode()
    }.value
    guard epoch == sessionEpoch, !Task.isCancelled, request > appliedRefreshSequence else { return }
    appliedRefreshSequence = request
    let decodedTelemetry = decoded.aged(by: telemetryElapsed(since: payloads.capturedAt))
    let snapshotElapsed = telemetryElapsed(since: snapshotCapturedAt)
    nextSnapshot.fleetAge = nextSnapshot.fleetAge.map { advancedTelemetryAge($0, by: snapshotElapsed) }
    nextSnapshot.rovers = nextSnapshot.rovers.map { value in
      var rover = value
      rover.poseAge = rover.poseAge.map { advancedTelemetryAge($0, by: snapshotElapsed) }
      rover.goalAge = rover.goalAge.map { advancedTelemetryAge($0, by: snapshotElapsed) }
      return rover
    }
    telemetryAppliedAt = .now
    snapshot = nextSnapshot
    occupancyStates = decodedTelemetry.occupancyStates
    let previous = operatorStates
    operatorStates = decodedTelemetry.operatorStates
    if let selected,
      previous[String(selected)]?.status?.revision
        != operatorStates[String(selected)]?.status?.revision
        || previous[String(selected)]?.status?.run_id
          != operatorStates[String(selected)]?.status?.run_id
    {
      clearInput()
    }

    let operatorData = decodedTelemetry.operatorTelemetry
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
    clouds = decodedTelemetry.clouds
    sceneCloudCache = sceneCloudCache.filter { clouds[$0.key] != nil }
    occupancyMaps = decodedTelemetry.occupancyMaps
    for (id, token) in autonomyPending where authorities[id]?.token == token {
      autonomyPending.removeValue(forKey: id)
    }
    localizations = Dictionary(
      uniqueKeysWithValues: decodedTelemetry.localizations.compactMap { key, value in
        UInt64(key).map { ($0, value) }
      })
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
    // Publish a complete presentation before yielding to report acknowledgements.
    // A slow command must not leave new safety state paired with old maps/clouds.
    let reports = applyReports(operatorData)
    await acknowledgeReports(reports, epoch: epoch, request: request)
  }
  private func applyReports(_ operatorData: [String: OperatorTelemetry])
    -> [(id: UInt64, report: TargetSearchReport)]
  {
    var reports: [(id: UInt64, report: TargetSearchReport)] = []
    for (key, data) in operatorData {
      guard let id = UInt64(key) else { continue }
      for report in data.reports ?? [] {
        if !(searchReports[id] ?? []).contains(where: {
          $0.runID == report.runID && $0.searchID == report.searchID
            && $0.reportID == report.reportID
        }) {
          var history = searchReports[id] ?? []
          history.append(report)
          searchReports[id] = Array(history.suffix(128))
        }
        reports.append((id, report))
      }
    }
    return reports
  }
  private func acknowledgeReports(_ reports: [(id: UInt64, report: TargetSearchReport)],
    epoch: UInt64, request: UInt64) async
  {
    for (id, report) in reports {
      guard epoch == sessionEpoch, !Task.isCancelled, request == appliedRefreshSequence else { return }
      let identity = "\(id):\(report.runID):\(report.searchID):\(report.reportID)"
      if reportAckTimes[identity].map({ Date().timeIntervalSince($0) < 1 }) == true { continue }
      let ack: [String: Any] = [
        "version": 1, "run_id": report.runID, "search_id": report.searchID,
        "report_id": report.reportID, "token": UUID().uuidString,
      ]
      if let payload = try? JSONSerialization.data(withJSONObject: ack) {
        do {
          try await backend.operatorCommand(
            id: id, kind: "search/report/ack", payload: String(decoding: payload, as: UTF8.self))
          guard epoch == sessionEpoch, !Task.isCancelled, request == appliedRefreshSequence else { return }
          reportAckTimes[identity] = Date()
        } catch {
          guard epoch == sessionEpoch, !Task.isCancelled, request == appliedRefreshSequence else { return }
          self.error = "Report acknowledgement failed · \(error.localizedDescription)"
        }
      }
    }
    reportAckTimes = reportAckTimes.filter { Date().timeIntervalSince($0.value) < 5 }
  }
  func motorsArmed(_ id: UInt64) -> Bool {
    guard let state = hardware[id] else { return false }
    let age = advancedTelemetryAge(state.age, by: elapsedSinceTelemetry)
    return age.isFinite && age >= 0 && age < 0.5 && state.armed
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
    let elapsed = elapsedSinceTelemetry
    guard let authority = authorities[id], authority.fresh,
      advancedTelemetryAge(authority.age, by: elapsed) < 0.5 else { return false }
    return snapshot.link == "healthy"
      && snapshot.rovers.contains {
        $0.id == id && $0.membership == "online"
          && $0.poseAge.map {
            let age = advancedTelemetryAge($0, by: elapsed)
            return age.isFinite && age >= 0 && age < 0.5
          } == true
      }
  }
  func waypointAvailable(_ id: UInt64) -> Bool {
    let elapsed = elapsedSinceTelemetry
    guard snapshot.link == "healthy", let authority = authorities[id],
      SupervisionTiming.usable(advancedTelemetryAge(authority.age, by: elapsed)), let hardware = hardware[id],
      SupervisionTiming.usable(advancedTelemetryAge(hardware.age, by: elapsed)), hardware.ready,
      hardware.armed else { return false }
    return snapshot.rovers.contains {
      $0.id == id && $0.membership == "online"
        && $0.poseAge.map { SupervisionTiming.usable(advancedTelemetryAge($0, by: elapsed)) } == true
    }
  }
  func selectAutonomy(id: UInt64, level: String) async -> String? {
    guard (["waypoint", "waypoint_direct"].contains(level) ? waypointAvailable(id) : driveAvailable(id)), authorities[id]?.supportedLevels.contains(level) == true else {
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
    guard let map = sceneOccupancy(id), map.usable else { return nil }
    var local = (x: x, y: y)
    if let origin = geographicOrigin {
      guard let ref = localizations[id], ref.usable else { return nil }
      local = ref.local(x: x, y: y, origin: origin)
    }
    return map.value(x: local.x, y: local.y)
  }
  func sceneOccupancy(_ id: UInt64) -> OccupancyFrame? {
    guard var map = occupancyMaps[id] else { return nil }
    map.age = advancedTelemetryAge(map.age, by: elapsedSinceTelemetry)
    guard map.age.isFinite, map.age >= 0, map.age < 0.5 else { return nil }
    return map
  }
  func sceneCloud(_ id: UInt64) -> PointCloudFrame? {
    guard var cloud = clouds[id] else { return nil }
    cloud.age = advancedTelemetryAge(cloud.age, by: elapsedSinceTelemetry)
    guard cloud.fresh else { return nil }
    if let ref = localizations[id], cloud.frameID != ref.frameID { return nil }
    var transform: [Double] = []
    if let origin = geographicOrigin {
      guard let ref = localizations[id], ref.usable, cloud.frameID == ref.frameID else {
        return nil
      }
      transform = [origin.latitude, origin.longitude, ref.originLatitude!, ref.originLongitude!,
        ref.anchorX!, ref.anchorY!, ref.rotation!]
    }
    let placement = CloudPlacement(sequence: cloud.sequence, sourceFrame: cloud.frameID,
      sceneFrame: frameKey, transform: transform)
    if let cached = sceneCloudCache[id], cached.placement == placement {
      var result = cached.cloud
      result.age = cloud.age
      return result
    }
    if let origin = geographicOrigin, let ref = localizations[id] {
      cloud.points = cloud.points.map { point in
        let transformed = ref.scene(x: point[0], y: point[1], origin: origin)
        return [transformed.x, transformed.y, point[2]]
      }
    }
    cloud.frameID += ":\(id):\(sessionEpoch):" + frameKey
    if !transform.isEmpty {
      // A changed alignment must reset history even if the source sequence did
      // not change; otherwise points from two coordinate systems are mixed.
      cloud.frameID += ":" + transform.map(String.init(describing:)).joined(separator: ",")
    }
    sceneCloudCache[id] = (placement, cloud)
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
      let waypointLevel = authorities[id]?.requestedLevel == "waypoint_direct" ? "waypoint_direct" : "waypoint"
      if authorities[id]?.requestedLevel != waypointLevel {
        guard let token = await selectAutonomy(id: id, level: waypointLevel) else { return false }
        let deadline = Date().addingTimeInterval(SupervisionTiming.acknowledgmentTimeout)
        var accepted = false
        while Date() < deadline {
          guard operation == waypointOperation, epoch == sessionEpoch, !Task.isCancelled else { return false }
          await refresh()
          guard operation == waypointOperation, epoch == sessionEpoch, !Task.isCancelled else { return false }
          if let authority = authorities[id], authority.token == token {
            guard authority.result == "accepted", authority.requestedLevel == waypointLevel else {
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
