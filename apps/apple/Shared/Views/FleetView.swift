import MapKit
import SwiftUI

struct FleetView: View {
  @EnvironmentObject var model: FleetModel
  @Environment(\.scenePhase) private var scenePhase
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @AppStorage("endpoint") private var endpoint = "tcp/127.0.0.1:7448"
  @AppStorage("prefix") private var prefix = "terra/phone"
  @StateObject private var drive = RoverDriveController()
  @StateObject private var renderer = RoverSceneController()
  @State private var missions = false
  @State private var settings = false
  @State private var camera = FieldCamera()
  @State private var dragCamera: FieldCamera?
  @State private var zoomExtent: Double?
  @State private var sceneState = FleetSceneState()
  private var focused: UInt64? { get { sceneState.selectedID } nonmutating set { sceneState.select(newValue) } }
  private var draft: (Double, Double)? {
    get { sceneState.draft.map { ($0.x, $0.y) } }
    nonmutating set {
      let point = newValue.map { ScenePoint(x: $0.0, y: $0.1) }
      if point != sceneState.draft { model.cancelPendingWaypoint() }
      sceneState.draft = point
    }
  }
  @State private var acknowledged: String?
  private var vehicle: RoverView? { model.sceneRovers.first { $0.id == focused } }
  private let ink = Color(red: 0.75, green: 0.92, blue: 0.96)
  private let accent = Color.cyan
  private let surface = Color(red: 0.025, green: 0.055, blue: 0.085)
  @State private var searchClass = "person"
  @State private var searchMinX = "-20"
  @State private var searchMinY = "-20"
  @State private var searchMaxX = "20"
  @State private var searchMaxY = "20"
  @State private var searchBudget = "600"
  var body: some View {
    VStack(spacing: 0) {
      topBar
      GeometryReader { geometry in
        ZStack(alignment: .topLeading) {
          let edgePositions = FieldMarkerPlacement.locations(
            desired: model.sceneRovers.compactMap { rover in
              if let pose = rover.pose {
                let p = camera.screen(x: pose.x, y: pose.y, size: geometry.size)
                return visible(p, size: geometry.size) ? nil : (rover.id, p)
              }
              return (rover.id, CGPoint(x: geometry.size.width / 2, y: geometry.size.height))
            }, size: geometry.size)
          if let origin = model.geographicOrigin, !camera.dimensional {
            Map(initialPosition: .region(mapRegion(origin: origin, size: geometry.size)))
              .mapStyle(.standard(elevation: .flat)).allowsHitTesting(false)
              .id(
                "\(camera.x):\(camera.y):\(camera.extent):\(geometry.size.width):\(geometry.size.height):\(model.frameKey)"
              )
          }
          ground(size: geometry.size)
            .contentShape(Rectangle())
            .onTapGesture { point in tap(point, size: geometry.size) }
            .gesture(
              DragGesture(minimumDistance: 8).onChanged { value in
                if dragCamera == nil { dragCamera = camera }
                guard let original = dragCamera else { return }
                let center = CGPoint(x: geometry.size.width / 2 - value.translation.width, y: geometry.size.height / 2 - value.translation.height)
                let point = original.world(center, size: geometry.size)
                camera.x = point.x; camera.y = point.y
              }.onEnded { _ in dragCamera = nil }
            )
            .simultaneousGesture(
              MagnificationGesture().onChanged { factor in
                if zoomExtent == nil { zoomExtent = camera.extent }
                camera.extent = max(3, min(10000, (zoomExtent ?? 30) / factor))
              }.onEnded { _ in zoomExtent = nil })
          RoverScene(
            controller: renderer, rovers: model.sceneRovers, camera: camera, size: geometry.size,
            cloud: focused.flatMap { model.sceneCloud($0) }
          )
          .frame(width: geometry.size.width, height: geometry.size.height)
          .allowsHitTesting(false).accessibilityHidden(true)
          ForEach(model.sceneRovers, id: \.id) { rover in
            if let pose = rover.pose {
              let point = camera.screen(x: pose.x, y: pose.y, size: geometry.size)
              if visible(point, size: geometry.size) {
                if focused != rover.id {
                  Button {
                    focus(rover)
                  } label: {
                    robotNote(rover)
                  }
                  .buttonStyle(.plain).position(notePoint(point, size: geometry.size))
                  .accessibilityIdentifier("scene-rover-\(rover.id)").accessibilityLabel(
                    "Select Rover \(rover.id), \(DashboardPresentation.attentionReason(rover) ?? rover.goal?.state ?? "state unknown")"
                  )
                }
              } else {
                edgeMarker(rover, point: edgePositions[rover.id] ?? point, size: geometry.size)
              }
            } else {
              unknownMarker(rover, point: edgePositions[rover.id] ?? CGPoint(x: 80, y: 100))
            }
          }
          if let vehicle {
            let anchor =
              vehicle.pose.map { camera.screen(x: $0.x, y: $0.y, size: geometry.size) }
              ?? CGPoint(x: geometry.size.width / 2, y: geometry.size.height / 2)
            let contentHeight =
              540.0 + (draft == nil ? 0 : 80)
              + (DashboardPresentation.attentionReason(vehicle) == nil ? 0 : 55)
              + (model.snapshot.notice == nil ? 0 : 45)
            let panelHeight = max(120, min(contentHeight, geometry.size.height - 80))
            if sceneState.inspecting || draft != nil {
            ScrollView { robotControls(vehicle) }
              .frame(width: min(310, geometry.size.width - 32), height: panelHeight)
              .background(surface.opacity(0.96)).overlay { RoundedRectangle(cornerRadius: 12).stroke(accent.opacity(0.4), lineWidth: 1) }
              .position(controlPoint(anchor, size: geometry.size, height: panelHeight))
            } else {
              VStack(alignment: .leading, spacing: 8) {
                Text("T-\(vehicle.id)").font(.headline)
                Text(task(vehicle)).font(.caption)
                if let reason = DashboardPresentation.attentionReason(vehicle) {
                  Label(reason, systemImage: "exclamationmark.triangle").font(.caption).foregroundStyle(.orange)
                }
                Button("Inspect & controls") { sceneState.inspecting = true }
                  .accessibilityIdentifier("inspect-rover")
              }.padding(12).background(surface.opacity(0.9), in: RoundedRectangle(cornerRadius: 12))
                .position(notePoint(anchor, size: geometry.size))
            }
          }
          VStack(alignment: .leading, spacing: 5) {
            Text(focused == nil ? "THE PLACE" : "AROUND THIS ROBOT").font(
              .system(size: 10, weight: .semibold)
            ).tracking(1)
            Text(
              focused == nil
                ? "Pan, zoom, and tap a robot." : "Outside the robot · reported pose only"
            ).font(.caption)
            if let focused {
              Text(
                model.clouds[focused].map {
                  "\($0.source == "lidar" ? "Depth" : "Sparse feature") cloud · \($0.points.count) points"
                } ?? "Waiting for point cloud"
              ).font(.caption).foregroundStyle(.secondary)
            }
          }.padding(14).allowsHitTesting(false)
          if model.sceneRovers.isEmpty {
            VStack(spacing: 12) {
              Image(systemName: "view.3d").font(.system(size: 32, weight: .light))
              Text(
                model.snapshot.link == "disconnected"
                  ? "Connect to bring the robots into view"
                  : "Waiting for the phone's fleet telemetry")
              Button("Connection settings") { settings = true }.buttonStyle(.bordered)
            }.font(.subheadline).position(x: geometry.size.width / 2, y: geometry.size.height / 2)
          }
          VStack {
            Spacer()
            HStack(spacing: 8) {
              Text(
                "\(model.geographicOrigin == nil ? "LOCAL POSITIONING" : "GEOGRAPHIC POSITIONING") · \(Int(camera.extent * 2)) m across short axis"
              ).font(.system(size: 10)).foregroundStyle(.secondary)
              Spacer()
              Button {
                camera.extent = min(10000, camera.extent * 1.5)
              } label: {
                Image(systemName: "minus")
              }.accessibilityLabel("Zoom out")
              Button {
                camera.extent = max(3, camera.extent / 1.5)
              } label: {
                Image(systemName: "plus")
              }.accessibilityLabel("Zoom in")
            }.padding(12)
          }.buttonStyle(.bordered)
        }.clipped().background(surface)
      }
      bottomBar
    }.foregroundStyle(ink).background(surface).preferredColorScheme(.dark).tint(accent)
      .sheet(isPresented: $missions) { FleetMissionView() }
      .sheet(isPresented: $settings) { ConnectionView() }
      .alert(
        "Operator notice",
        isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })
      ) {
        Button("OK") { model.error = nil }
      } message: {
        Text(model.error ?? "")
      }
      .onChange(of: scenePhase) { _, phase in
        if phase == .active {
          model.startObservation()
        } else {
          drive.stop()
          model.clearInput()
          model.stopObservation()
        }
      }
      .onChange(of: missions) { _, _ in
        drive.stop()
        model.clearInput()
      }
      .onChange(of: settings) { _, shown in if shown { drive.stop() } }
      .onAppear { drive.attach(model) }
      .onDisappear { drive.detach() }
      .onChange(of: model.sceneRovers.map(\.id)) { _, ids in
        if let focused, !ids.contains(focused) { wider() }
      }
      .onChange(of: model.sessionEpoch) { _, _ in wider() }
      .onChange(of: model.frameKey) { _, _ in wider() }
      .task {
        if ProcessInfo.processInfo.arguments.contains("--connect") {
          await model.connect(endpoint: endpoint, prefix: prefix)
        }
      }
  }
  private var topBar: some View {
    HStack(spacing: 12) {
      Text("ARGOS").font(.system(size: 14, weight: .bold)).tracking(2)
      Text(focused == nil ? "The place" : "Around this robot").font(.caption)
      if focused != nil { Button("Wider") { wider() }.buttonStyle(.bordered) }
      if camera.dimensional {
        Button { camera.azimuth -= .pi / 8 } label: { Image(systemName: "rotate.left") }.accessibilityLabel("Orbit left")
        Button { camera.azimuth += .pi / 8 } label: { Image(systemName: "rotate.right") }.accessibilityLabel("Orbit right")
      }
      Picker("Camera", selection: $camera.dimensional) {
        Text("2D").tag(false)
        Text("3D").tag(true)
      }
      .pickerStyle(.segmented).labelsHidden().frame(width: 90).accessibilityLabel("Scene camera")
      Spacer(minLength: 0)
      Button {
        settings = true
      } label: {
        Image(systemName: "network")
      }.buttonStyle(.plain).accessibilityLabel("Connection settings").accessibilityIdentifier(
        "connection")
      Text(model.snapshot.link == "healthy" ? "Connected" : model.snapshot.link.capitalized).font(
        .caption)
      Button { missions = true } label: { Image(systemName: "flag") }.accessibilityLabel("Mission controls")
      Button("STOP ALL") {
        drive.stop()
        model.clearInput()
        Task { await model.fleetAction(stop: true) }
      }.buttonStyle(.borderedProminent).disabled(model.snapshot.link == "disconnected")
    }.padding(.horizontal, 14).padding(.vertical, 10)
      .overlay(alignment: .bottom) { Rectangle().fill(ink).frame(height: 1) }
  }
  private var bottomBar: some View {
    HStack(spacing: 12) {
      Label("FLEET", systemImage: "view.3d").font(.system(size: 11, weight: .semibold))
      Text(
        focused == nil
          ? "Select a robot to inspect its reported state."
          : "Tap the ground to draft a waypoint · dashed line marks a target, not a planned route."
      )
      .font(.caption).frame(maxWidth: .infinity, alignment: .leading)
      Text(
        model.operatorStates.values.contains(where: { $0.recording == "incomplete" })
          ? "Recording incomplete"
          : model.operatorStates.isEmpty ? "Waiting for session status" : "Session recording"
      ).font(.system(size: 10)).foregroundStyle(.secondary)
    }.padding(12).overlay(alignment: .top) { Rectangle().fill(ink).frame(height: 1) }
  }
  private func robotNote(_ rover: RoverView) -> some View {
    VStack(alignment: .leading, spacing: 4) {
      Text("T-\(rover.id)").fontWeight(.semibold)
      if let reason = DashboardPresentation.attentionReason(rover) {
        Text("CHECK · \(reason)").fontWeight(.semibold)
      }
    }.font(.system(size: 11)).padding(8).background(surface.opacity(0.75), in: RoundedRectangle(cornerRadius: 8)).overlay {
      RoundedRectangle(cornerRadius: 8).stroke(accent.opacity(0.25), lineWidth: 1)
    }
  }
  private func robotControls(_ rover: RoverView) -> some View {
    VStack(alignment: .leading, spacing: 12) {
      HStack {
        Text("T-\(rover.id) · \(rover.membership)").font(.headline)
        Spacer()
        Button {
          wider()
        } label: {
          Image(systemName: "xmark")
        }.buttonStyle(.plain).accessibilityLabel("Return to the place")
      }
      Text(task(rover)).font(.subheadline)
      Text(
        rover.pose.map {
          String(
            format:
              "\(model.geographicOrigin == nil ? "Local" : "North / west") %.2f, %.2f m · pose %.1f s",
            $0.x, $0.y, rover.poseAge ?? 0)
        } ?? "Position unavailable · location in scene unknown"
      ).font(.caption).foregroundStyle(.secondary)
      Text(model.localizations[rover.id]?.reason ?? "Geographic alignment unavailable").font(
        .caption
      ).foregroundStyle(.secondary)
      autonomyControls(rover)
      searchControls(rover)
      HStack {
        Text(
          model.hardware[rover.id].map {
            $0.simulated == true
              ? "Simulated actuation ready"
              : $0.armed ? "Motors armed" : $0.arming ? "Arming…" : $0.reason
          } ?? "Hardware status unavailable"
        ).font(.caption)
        Spacer()
        Button(model.hardwarePending[rover.id] != nil ? "Arm requested…" : "Arm") {
          Task { await model.requestHardware(id: rover.id, arm: true) }
        }.buttonStyle(.bordered)
          .disabled(
            model.hardware[rover.id]?.simulated == true || model.hardwarePending[rover.id] != nil
              || model.hardware[rover.id]?.armed == true || model.hardware[rover.id]?.arming == true
              || !model.driveAvailable(rover.id) || model.hardware[rover.id]?.ready != true
          )
          .accessibilityIdentifier("hardware-arm")
        Button("Disarm") {
          drive.stop()
          Task { await model.requestHardware(id: rover.id, arm: false) }
        }.buttonStyle(.bordered).disabled(model.hardware[rover.id]?.simulated == true)
          .accessibilityIdentifier("hardware-disarm")
      }
      if model.hardwarePending[rover.id] != nil {
        Text("Waiting for rover arm confirmation…").font(.caption)
      }
      HStack(alignment: .center, spacing: 12) {
        RoverJoystick(drive: drive)
        VStack(alignment: .leading, spacing: 8) {
          Button(drive.enabled || drive.requesting ? "Stop driving" : "Drive this rover") {
            if drive.enabled || drive.requesting {
              drive.stop()
            } else {
              draft = nil
              Task { await drive.start(id: rover.id) }
            }
          }.buttonStyle(.borderedProminent).disabled(
            !drive.enabled && !drive.requesting
              && (!model.driveAvailable(rover.id) || !model.motorsArmed(rover.id)))
          Text("Max 0.5 m/s · 1 rad/s").font(.system(size: 10)).foregroundStyle(.secondary)
          if let name = drive.gamepadName { Text(name).font(.caption) }
        }
      }
      Text(drive.status).font(.caption).foregroundStyle(.secondary)
      Text(
        model.occupancyMaps[rover.id].map {
          "Live occupancy · \($0.width)×\($0.height) · \(String(format: "%.2f", $0.resolution)) m cells"
        } ?? "Occupancy unavailable or stale"
      ).font(.caption)
      Text("Pale: observed free · dark: occupied · hatched: unknown").font(.system(size: 10))
        .foregroundStyle(.secondary)
      if let notice = model.snapshot.notice { Text(notice).font(.caption) }
      if let reason = DashboardPresentation.attentionReason(rover) {
        Text("CHECK · \(reason)").font(.caption.weight(.semibold))
        Button(acknowledged == "\(rover.id):\(reason)" ? "Acknowledged" : "Acknowledge") {
          acknowledged = "\(rover.id):\(reason)"
        }.controlSize(.small)
      }
      Divider()
      if let draft {
        Text("DIRECTED ORDER").font(.system(size: 10, weight: .semibold)).tracking(1)
        Text(String(format: "Go to %.2f, %.2f m", draft.0, draft.1)).font(.subheadline)
          .monospacedDigit()
        Text(waypointContext(id: rover.id, draft: draft)).font(.caption).foregroundStyle(.secondary)
        HStack {
          Button("Confirm waypoint") {
            Task {
              let sent = await model.send(id: rover.id, waypoint: waypoint(id: rover.id, draft: draft))
              if sent, focused == rover.id, self.draft?.0 == draft.0, self.draft?.1 == draft.1 { self.draft = nil }
            }
          }
          .buttonStyle(.borderedProminent).disabled(
            !model.waypointAvailable(rover.id) || !canCommand(rover)
              || (model.authorities[rover.id]?.requestedLevel != "waypoint_direct" && (model.waypointCell(id: rover.id, x: draft.0, y: draft.1) ?? -1) >= 65)
          ).accessibilityIdentifier("send-waypoint")
          Button("Discard") { model.cancelPendingWaypoint(); self.draft = nil }.buttonStyle(.bordered)
        }
      } else {
        Text("Tap a point on the ground to draft a waypoint.").font(.caption).foregroundStyle(
          .secondary)
      }
      HStack {
        Button("Cancel goal", role: .destructive) { Task { await model.cancel(id: rover.id) } }
          .buttonStyle(.bordered).disabled(
            rover.membership != "online" || model.snapshot.link != "healthy" || (model.busy && model.pendingWaypointID != rover.id)
              || rover.commandPhase == "cancelling"
          ).accessibilityIdentifier("cancel-goal")
        EmptyView()
      }
      if rover.commandPhase != "none" {
        Text("Command · \(rover.commandPhase)").font(.caption).accessibilityIdentifier(
          "command-phase")
      }
      Text("Cancel clears a waypoint; physical stop is unavailable.").font(.system(size: 10))
        .foregroundStyle(.secondary)
    }.padding(16)
  }
  private func waypointContext(id: UInt64, draft: (Double, Double)) -> String {
    if model.authorities[id]?.requestedLevel == "waypoint_direct" { return "L2 direct waypoint · obstacles will not be avoided" }
    guard let value = model.waypointCell(id: id, x: draft.0, y: draft.1) else {
      return "Outside a fresh observed map · clearance unknown"
    }
    if value < 0 { return "Unknown cell · clearance not observed" }
    if value >= 65 { return "Occupied cell · choose another waypoint" }
    return value <= 35
      ? "Observed free cell · rover checks the route"
      : "Uncertain occupancy · inspect before confirming"
  }
  private func autonomyControls(_ rover: RoverView) -> some View {
    VStack(alignment: .leading, spacing: 6) {
      Text("AUTONOMY").font(.system(size: 10, weight: .semibold)).tracking(1)
      if let authority = model.authorities[rover.id] {
        Text(
          "Reported: \(authority.requestedLevel.replacingOccurrences(of: "_", with: " ")) · \(authority.reason)"
        ).font(.caption)
        if !authority.fresh { Text("Authority is stale").font(.caption) }
        if model.autonomyPending[rover.id] != nil {
          Text("Waiting for vehicle acknowledgement…").font(.caption)
        }
        if authority.result == "rejected" {
          Text("Vehicle rejected the last request").font(.caption)
        }
      } else {
        Text("Waiting for vehicle authority").font(.caption)
      }
      LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 5) {
        ForEach(AutonomyChoice.all) { choice in
          Button {
            guard let wire = choice.wire else { return }
            drive.stop()
            draft = nil
            Task { _ = await model.selectAutonomy(id: rover.id, level: wire) }
          } label: {
            Text("L\(choice.id) · \(choice.title)").lineLimit(2).fixedSize(
              horizontal: false, vertical: true
            ).frame(maxWidth: .infinity, minHeight: 28, alignment: .leading)
          }.font(.system(size: 10)).buttonStyle(.bordered)
            .disabled(
              !model.driveAvailable(rover.id)
                || choice.wire.map {
                  model.authorities[rover.id]?.supportedLevels.contains($0) != true
                } != false
            )
            .help(
              choice.wire == nil
                ? "This autonomy behavior is not implemented on the rover."
                : "Request this level; wait for vehicle acknowledgement.")
        }
      }
      Text("L2 has no obstacle avoidance. L4 requires a LiDAR phone with live person detection. L5 is unavailable.").font(.system(size: 10))
        .foregroundStyle(.secondary)
    }
  }
  @ViewBuilder private func searchControls(_ rover: RoverView) -> some View {
    if model.authorities[rover.id]?.requestedLevel == "target_search"
      || model.searches[rover.id] != nil || !(model.searchReports[rover.id] ?? []).isEmpty
    {
      VStack(alignment: .leading, spacing: 8) {
        Text("TARGET SEARCH").font(.system(size: 10, weight: .semibold)).tracking(1)
        TextField("Target class", text: $searchClass).textFieldStyle(.roundedBorder)
          .accessibilityLabel("Target class")
        Text("Search bounds · rover local metres").font(.caption).foregroundStyle(.secondary)
        HStack {
          TextField("Min x", text: $searchMinX).accessibilityLabel("Search minimum x")
          TextField("Max x", text: $searchMaxX).accessibilityLabel("Search maximum x")
        }.textFieldStyle(.roundedBorder)
        HStack {
          TextField("Min y", text: $searchMinY).accessibilityLabel("Search minimum y")
          TextField("Max y", text: $searchMaxY).accessibilityLabel("Search maximum y")
        }.textFieldStyle(.roundedBorder)
        TextField("Time budget · seconds", text: $searchBudget).textFieldStyle(.roundedBorder)
          .accessibilityLabel("Search time budget in seconds")
        Button("Start search") {
          guard let a = Double(searchMinX), let b = Double(searchMinY), let c = Double(searchMaxX),
            let d = Double(searchMaxY), let t = Double(searchBudget)
          else {
            model.error = "Enter numeric bounds and time budget"
            return
          }
          drive.stop()
          Task {
            await model.startSearch(
              id: rover.id, targetClass: searchClass, minX: a, minY: b, maxX: c, maxY: d, budget: t)
          }
        }.buttonStyle(.borderedProminent).disabled(
          !model.driveAvailable(rover.id)
            || model.authorities[rover.id]?.requestedLevel != "target_search"
            || model.searchPending[rover.id] != nil
            || model.searches[rover.id].map { !$0.terminal } == true
        ).accessibilityIdentifier("start-search")
        if model.searchPending[rover.id] != nil {
          Text("Waiting for rover acknowledgement…").font(.caption)
        }
        if let reports = model.searchReports[rover.id], !reports.isEmpty {
          DisclosureGroup("Confirmed reports · \(reports.count)") {
            ForEach(Array(reports.enumerated()), id: \.offset) { _, report in
              Text(
                String(
                  format: "%@ · %.2f, %.2f m · %d frames", report.targetClass, report.worldX,
                  report.worldY, report.frameIDs.count)
              ).font(.caption)
            }
          }
        }

        if let search = model.searches[rover.id] {
          Text(
            "\(search.phase.replacingOccurrences(of:"_",with:" ").capitalized) · \(search.reason)"
          ).font(.caption)
          Text(String(format: "Elapsed %.1f s", search.elapsed)).font(.caption).monospacedDigit()
          if !search.fresh {
            Text("Search status is stale").font(.caption).foregroundStyle(.orange)
          }
          if let report = search.report {
            Text(String(format: "Target confirmed at %.2f, %.2f m", report.worldX, report.worldY))
              .font(.caption)
            DisclosureGroup("Detection evidence") {
              Text("\(report.frameIDs.count) frames · \(report.evidenceIDs.joined(separator:", "))")
                .font(.caption).textSelection(.enabled)
            }
          }
          HStack {
            Button("Pause") { Task { await model.searchAction(id: rover.id, action: "pause") } }
              .disabled(search.terminal || search.phase == "paused")
            Button("Resume") { Task { await model.searchAction(id: rover.id, action: "resume") } }
              .disabled(!["paused", "needs_attention"].contains(search.phase))
            Button("Cancel", role: .destructive) {
              Task { await model.searchAction(id: rover.id, action: "cancel") }
            }.disabled(search.terminal)
          }.buttonStyle(.bordered).disabled(!search.fresh || model.searchPending[rover.id] != nil)
        }
      }
    }
  }
  private func waypoint(id: UInt64, draft: (Double, Double)) -> Waypoint {
    if let origin = model.geographicOrigin, let ref = model.localizations[id], ref.usable {
      let local = ref.local(x: draft.0, y: draft.1, origin: origin)
      return .local(x: local.x, y: local.y, yaw: nil)
    }
    return .local(x: draft.0, y: draft.1, yaw: nil)
  }
  private func mapRegion(origin: CLLocationCoordinate2D, size: CGSize) -> MKCoordinateRegion {
    let center = MapProjection.coordinate(
      x: camera.x, y: camera.y, latitude: origin.latitude, longitude: origin.longitude)
    let scale = camera.scale(size)
    return MKCoordinateRegion(
      center: center,
      span: MKCoordinateSpan(
        latitudeDelta: size.height / scale / MapProjection.scale,
        longitudeDelta: size.width / scale
          / (MapProjection.scale * cos(center.latitude * .pi / 180))))
  }
  private func canCommand(_ rover: RoverView) -> Bool {
    rover.membership == "online" && model.snapshot.link == "healthy" && !model.busy
      && !["pending", "cancelling"].contains(rover.commandPhase)
  }
  private func task(_ rover: RoverView) -> String {
    guard let goal = rover.goal else { return "Task not reported" }
    if goal.state == "active" {
      return String(format: "Waypoint · %.1f m remaining", goal.distance)
    }
    return goal.state == "arrived" ? "Waypoint reached" : "Idle"
  }
  private func focus(_ rover: RoverView) {
    drive.stop()
    camera.dimensional = true
    focused = rover.id
    model.selected = rover.id
    draft = nil
    if let pose = rover.pose {
      withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.3)) {
        camera.x = pose.x
        camera.y = pose.y
        camera.extent = 5
      }
    }
  }
  private func wider() {
    drive.stop()
    focused = nil
    model.selected = nil
    draft = nil
    sceneState.inspecting = false
    camera.azimuth = 0
    withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.3)) {
      camera.x = 0
      camera.y = 0
      camera.extent = 30
    }
  }
  private func visible(_ point: CGPoint, size: CGSize) -> Bool {
    point.x >= 45 && point.x <= size.width - 45 && point.y >= 70 && point.y <= size.height - 70
  }
  private func notePoint(_ point: CGPoint, size: CGSize) -> CGPoint {
    CGPoint(
      x: min(size.width - 105, max(105, point.x + 115)),
      y: min(size.height - 80, max(80, point.y - 70)))
  }
  private func controlPoint(_ point: CGPoint, size: CGSize, height: Double) -> CGPoint {
    let halfHeight = CGFloat(height) / 2
    return CGPoint(
      x: min(size.width - 170, max(170, point.x + 225)),
      y: min(size.height - halfHeight - 12, max(halfHeight + 12, point.y - 30)))
  }
  private func edgeMarker(_ rover: RoverView, point: CGPoint, size: CGSize) -> some View {
    Button {
      focus(rover)
    } label: {
      Label(
        "T-\(rover.id)\(DashboardPresentation.attentionReason(rover) == nil ? "" : " · CHECK")",
        systemImage: "arrow.up.right")
    }
    .font(.caption).buttonStyle(.borderedProminent)
    .position(x: min(size.width - 65, max(65, point.x)), y: min(size.height - 55, max(70, point.y)))
    .accessibilityLabel("Locate Rover \(rover.id)")
  }
  private func unknownMarker(_ rover: RoverView, point: CGPoint) -> some View {
    Button {
      focus(rover)
    } label: {
      Label("T-\(rover.id) · no position", systemImage: "location.slash")
    }
    .font(.caption).buttonStyle(.bordered).position(point)
    .accessibilityIdentifier("scene-rover-\(rover.id)")
    .accessibilityLabel("Inspect Rover \(rover.id), position unavailable")
  }
  private func tap(_ point: CGPoint, size: CGSize) {
    if let id = renderer.rover(at: point),
      let rover = model.sceneRovers.first(where: { $0.id == id })
    {
      focus(rover)
      return
    }

    let nearest = model.sceneRovers.compactMap { rover -> (RoverView, Double)? in
      guard let pose = rover.pose else { return nil }
      let p = camera.screen(x: pose.x, y: pose.y, size: size)
      return (rover, hypot(p.x - point.x, p.y - point.y))
    }.min { $0.1 < $1.1 }
    if let nearest, nearest.1 < 30 {
      focus(nearest.0)
      return
    }
    guard focused != nil, !drive.enabled else { return }
    let world = camera.world(point, size: size)
    draft = (world.x, world.y)
  }
  private func ground(size: CGSize) -> some View {
    Canvas { context, size in
      if let focused, let map = model.occupancyMaps[focused], map.usable {
        var free = Path()
        var occupied = Path()
        var uncertain = Path()
        var unknown = Path()
        for row in 0..<map.height {
          for col in 0..<map.width {
            let value = map.occupancy[row * map.width + col]
            let x = map.originX + Double(col) * map.resolution
            let y = map.originY + Double(row) * map.resolution
            let corners = [
              (x, y), (x + map.resolution, y), (x + map.resolution, y + map.resolution),
              (x, y + map.resolution),
            ]
            let projected = corners.compactMap {
              model.mapScenePoint(id: focused, x: $0.0, y: $0.1)
            }.map { camera.screen(x: $0.x, y: $0.y, size: size) }
            guard projected.count == 4 else { continue }
            var cell = Path()
            cell.move(to: projected[0])
            projected.dropFirst().forEach { cell.addLine(to: $0) }
            cell.closeSubpath()
            if value < 0 {
              unknown.addPath(cell)
            } else if value >= 65 {
              occupied.addPath(cell)
            } else if value <= 35 {
              free.addPath(cell)
            } else {
              uncertain.addPath(cell)
            }
          }
        }
        context.fill(free, with: .color(.cyan.opacity(0.07)))
        context.fill(occupied, with: .color(.cyan.opacity(0.5)))
        context.fill(uncertain, with: .color(.orange.opacity(0.2)))
        context.fill(unknown, with: .color(.white.opacity(0.025)))
        context.stroke(
          unknown, with: .color(.white.opacity(0.1)),
          style: StrokeStyle(lineWidth: 0.4, dash: [1, 3]))
      }
      var grid = Path()
      let step = max(1, pow(10, floor(log10(camera.extent / 3))))
      let reach = camera.extent * 4
      let beginX = floor((camera.x - reach) / step) * step
      let beginY = floor((camera.y - reach) / step) * step
      for n in 0...Int(2 * reach / step) {
        let x = beginX + Double(n) * step
        let y = beginY + Double(n) * step
        grid.move(to: camera.screen(x: x, y: camera.y - reach, size: size))
        grid.addLine(to: camera.screen(x: x, y: camera.y + reach, size: size))
        grid.move(to: camera.screen(x: camera.x - reach, y: y, size: size))
        grid.addLine(to: camera.screen(x: camera.x + reach, y: y, size: size))
      }
      context.stroke(grid, with: .color(.cyan.opacity(0.11)), lineWidth: 1)
      for rover in model.sceneRovers {
        if let goal = rover.goal, goal.state != "idle", model.canPlaceGeographically(rover.id) {
          let p = camera.screen(x: goal.x, y: goal.y, size: size)
          context.draw(Text("⚑").font(.title2).foregroundStyle(ink), at: p)
          if let pose = rover.pose {
            var route = Path()
            route.move(to: camera.screen(x: pose.x, y: pose.y, size: size))
            route.addLine(to: p)
            context.stroke(
              route, with: .color(.cyan.opacity(0.65)),
              style: StrokeStyle(lineWidth: 1, dash: [5, 5]))
          }
        }
        if let pose = rover.pose {
          drawRobot(rover, x: pose.x, y: pose.y, yaw: pose.yaw, context: &context, size: size)
        }
      }
      if let draft {
        let p = camera.screen(x: draft.0, y: draft.1, size: size)
        context.stroke(
          Path(ellipseIn: CGRect(x: p.x - 10, y: p.y - 10, width: 20, height: 20)),
          with: .color(ink), lineWidth: 2)
        context.draw(
          Text("DRAFT").font(.system(size: 10, weight: .semibold)), at: CGPoint(x: p.x, y: p.y + 22)
        )
      }
    }.accessibilityIdentifier("field-scene").accessibilityLabel(
      "Shared robot scene. Pan and zoom. Select robot callouts to inspect and confirm directed waypoints."
    )
  }
  private func drawRobot(
    _ rover: RoverView, x: Double, y: Double, yaw: Double, context: inout GraphicsContext,
    size: CGSize
  ) {
    let point = camera.screen(x: x, y: y, size: size)
    if DashboardPresentation.attentionReason(rover) != nil {
      context.stroke(
        Path(ellipseIn: CGRect(x: point.x - 35, y: point.y - 22, width: 70, height: 44)),
        with: .color(.orange), style: StrokeStyle(lineWidth: 2, dash: [3, 3]))
    }
    if focused == rover.id {
      context.stroke(Path(ellipseIn: CGRect(x: point.x - 30, y: point.y - 19, width: 60, height: 38)), with: .color(accent), lineWidth: 2)
    }
    context.draw(
      Text("T-\(rover.id)").font(.system(size: 10, weight: .semibold)),
      at: CGPoint(x: point.x, y: point.y + 38))
  }
}

// Retained for the auxiliary geographic and detail views.
