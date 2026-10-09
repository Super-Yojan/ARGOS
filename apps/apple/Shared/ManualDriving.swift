import SwiftUI
import GameController
#if os(macOS)
import AppKit
#endif

struct DriveVector: Equatable {
    var linear: Double
    var angular: Double
    static let zero = DriveVector(linear: 0, angular: 0)
    static func axes(forward: Double, turn: Double) -> Self {
        func axis(_ v: Double) -> Double { v.isFinite && abs(v) > 0.12 ? max(-1, min(1, v)) : 0 }
        return Self(linear: axis(forward)*0.5, angular: axis(turn))
    }
    static func keyboard(_ keys: Set<String>) -> Self {
        axes(forward: (keys.contains("w") || keys.contains("up") ? 1.0 : 0) - (keys.contains("s") || keys.contains("down") ? 1.0 : 0),
             turn: (keys.contains("a") || keys.contains("left") ? 1.0 : 0) - (keys.contains("d") || keys.contains("right") ? 1.0 : 0))
    }
}
struct PointCloudFrame: Codable {
    var version: Int
    var sequence: UInt64
    var frameID: String
    var source: String
    var age: Double
    var points: [[Double]]
    var usable: Bool {
        version == 1 && !frameID.isEmpty && age.isFinite && age >= 0 && age < 2.5 && points.count <= 4096
        && points.allSatisfy { $0.count == 3 && $0.allSatisfy { $0.isFinite && abs($0) <= 20000 } }
    }
}
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
        case runID = "run_id", requestedLevel = "requested_level", effectiveLevel = "effective_level", activeSource = "active_source"
        case safety, reason, revision, token, result, supportedLevels = "supported_levels", age
    }
    var fresh: Bool { !runID.isEmpty && age.isFinite && age >= 0 && age < 0.5 }
    func acceptsDrive(token: String) -> Bool {
        fresh && self.token == token && result == "accepted" && ["teleop", "assisted_teleop"].contains(requestedLevel)
        && effectiveLevel == requestedLevel && ["clear", "active"].contains(safety)
    }
}
struct HardwareState: Codable {
    var simulated: Bool? = nil
    var ready: Bool
    var armed: Bool
    var arming: Bool
    var reason: String
    var token: String?
    var result: String?
    var age: Double
    var fresh: Bool { age.isFinite && age >= 0 && age < 0.5 }
}
struct OperatorTelemetry: Codable { var hardware: HardwareState?; var cloud: PointCloudFrame?; var authority: VehicleAuthority?; var occupancy: OccupancyFrame?; var search: TargetSearchState?; var reports: [TargetSearchReport]? }
struct AutonomyChoice: Identifiable {
    let id: Int; let title: String; let wire: String?
    static let all = [Self(id: 0, title: "Fully teleop", wire: "teleop"), Self(id: 1, title: "Assisted teleop", wire: "assisted_teleop"),
        Self(id: 2, title: "Waypoint · no avoidance", wire: nil), Self(id: 3, title: "Waypoint · obstacle aware", wire: "waypoint"),
        Self(id: 4, title: "Target search", wire: "target_search"), Self(id: 5, title: "Decision making", wire: nil)]
}

@MainActor final class RoverDriveController: ObservableObject {
    @Published private(set) var enabled = false
    @Published private(set) var requesting = false
    @Published private(set) var status = "Driving off"
    @Published private(set) var gamepadName: String?
    @Published var stick = CGPoint.zero
    private var joystickHeld = false
    private var keys = Set<String>()
    private var task: Task<Void, Never>?
    private weak var model: FleetModel?
    private var roverID: UInt64?
    private var token: String?
    private var session = UUID().uuidString
    private var sequence: UInt64 = 0
    private var active = false
    private var generation = UUID()
    #if os(macOS)
    private var keyboardMonitor: Any?
    private var focusObserver: NSObjectProtocol?
    #endif
    func attach(_ model: FleetModel) {
        self.model = model
        #if os(macOS)
        if keyboardMonitor == nil {
            keyboardMonitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .keyUp]) { [weak self] event in
                guard let self, self.enabled else { return event }
                let names: [UInt16: String] = [13:"w",1:"s",0:"a",2:"d",49:"space",126:"up",125:"down",123:"left",124:"right",53:"escape"]
                guard let key = names[event.keyCode] else { return event }
                if key == "escape" || event.modifierFlags.intersection([.command,.control,.option]).isEmpty == false { self.stop(); return event }
                if event.type == .keyDown { self.keys.insert(key) } else { self.keys.remove(key) }
                return nil
            }
            focusObserver = NotificationCenter.default.addObserver(forName: NSWindow.didResignKeyNotification, object: nil, queue: .main) { [weak self] _ in
                Task { @MainActor in self?.stop() }
            }
        }
        #endif
    }
    func start(id: UInt64) async {
        stop()
        guard let model, model.driveAvailable(id), model.motorsArmed(id) else { status = "Fresh rover authority required"; return }
        let generation = self.generation
        roverID = id; session = UUID().uuidString; sequence = 0; keys.removeAll(); joystickHeld = false; stick = .zero
        status = "Requesting takeover…"; active = true; requesting = true
        let level = model.authorities[id]?.requestedLevel == "assisted_teleop" ? "assisted_teleop" : "teleop"
        let receipt = await model.selectAutonomy(id: id, level: level)
        guard self.generation == generation, active, roverID == id else { return }
        guard let token = receipt else { active = false; requesting = false; status = "Takeover not confirmed"; return }
        self.token = token
        task = Task { [weak self] in
            guard let self else { return }
            let deadline = Date().addingTimeInterval(3)
            while !Task.isCancelled && self.active && self.generation == generation {
                guard let authority = model.authorities[id], model.driveAvailable(id) else { self.stop(reason: "Driving stopped · telemetry unavailable"); return }
                if authority.acceptsDrive(token: token) { break }
                if Date() >= deadline { self.stop(reason: "Takeover not acknowledged"); return }
                try? await Task.sleep(for: .milliseconds(100))
            }
            guard !Task.isCancelled, self.active, self.generation == generation else { return }
            self.requesting = false; self.enabled = true; self.status = "Hold WASD / arrows, shoulder + gamepad, or drag joystick"
            while !Task.isCancelled && self.active && self.generation == generation {
                guard let authority = model.authorities[id], authority.acceptsDrive(token: token), model.driveAvailable(id) else {
                    self.stop(reason: "Driving stopped · authority or tracking changed"); return
                }
                self.sequence += 1
                let vector = self.input()
                do { try await model.drive(id: id, vector: vector, authority: authority, session: self.session, sequence: self.sequence) }
                catch { if self.generation == generation { self.stop(reason: "Driving stopped · \(error.localizedDescription)") }; return }
                try? await Task.sleep(for: .milliseconds(100))
            }
        }
    }
    private func input() -> DriveVector {
        let pad = GCController.controllers().first { $0.extendedGamepad != nil }
        if gamepadName != pad?.vendorName { gamepadName = pad?.vendorName }
        #if !os(macOS)
        if let keyboard = GCKeyboard.coalesced?.keyboardInput {
            let map: [(GCKeyCode, String)] = [(.keyW,"w"),(.keyS,"s"),(.keyA,"a"),(.keyD,"d"),(.spacebar,"space"),(.upArrow,"up"),(.downArrow,"down"),(.leftArrow,"left"),(.rightArrow,"right")]
            keys = Set(map.filter { keyboard.button(forKeyCode: $0.0)?.isPressed == true }.map { $0.1 })
        } else { keys.removeAll() }
        #endif
        if joystickHeld { return .axes(forward: -stick.y, turn: -stick.x) }
        if !keys.intersection(["w","a","s","d","up","down","left","right"]).isEmpty { return .keyboard(keys) }
        if let pad = pad?.extendedGamepad, pad.leftShoulder.isPressed {
            return .axes(forward: Double(pad.leftThumbstick.yAxis.value), turn: -Double(pad.leftThumbstick.xAxis.value))
        }
        return .zero
    }
    func joystick(_ point: CGPoint?) {
        guard enabled else { return }
        joystickHeld = point != nil
        stick = point ?? .zero
    }
    func stop(reason: String = "Driving off") {
        generation = UUID(); active = false; enabled = false; requesting = false; task?.cancel(); task = nil; keys.removeAll(); joystickHeld = false; stick = .zero; status = reason
        if let model, let id = roverID, let authority = model.authorities[id] {
            sequence += 1
            let sequence = sequence, session = session
            Task { try? await model.drive(id: id, vector: .zero, authority: authority, session: session, sequence: sequence) }
        }
        roverID = nil; token = nil
    }
    func detach() {
        stop()
        #if os(macOS)
        if let keyboardMonitor { NSEvent.removeMonitor(keyboardMonitor) }; keyboardMonitor = nil
        if let focusObserver { NotificationCenter.default.removeObserver(focusObserver) }; focusObserver = nil
        #endif
    }
}
struct RoverJoystick: View {
    @ObservedObject var drive: RoverDriveController
    var body: some View {
        ZStack {
            Circle().stroke(.secondary, lineWidth: 1)
            Image(systemName: "plus").foregroundStyle(.secondary)
            Circle().fill(.primary).frame(width: 24, height: 24).offset(x: drive.stick.x*36, y: drive.stick.y*36)
        }.frame(width: 110, height: 110).contentShape(Circle())
            .gesture(DragGesture(minimumDistance: 0).onChanged { value in
                let x = (value.location.x-55)/42, y = (value.location.y-55)/42, length = max(1, hypot(x,y))
                drive.joystick(CGPoint(x:x/length, y:y/length))
            }.onEnded { _ in drive.joystick(nil) })
            .opacity(drive.enabled ? 1 : 0.3).allowsHitTesting(drive.enabled)
            .accessibilityLabel("Driving joystick; hold and drag to move, release to stop")
            .accessibilityIdentifier("drive-joystick")
    }
}

struct OccupancyFrame: Codable {
    var schemaVersion: Int
    var roverID: UInt64
    var runID: String
    var sequence: UInt64
    var width: Int
    var height: Int
    var resolution: Double
    var originX: Double
    var originY: Double
    var occupancy: [Int]
    var age: Double
    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version", roverID = "rover_id", runID = "run_id", sequence, width, height, resolution
        case originX = "origin_x", originY = "origin_y", occupancy, age
    }
    var usable: Bool {
        schemaVersion == 1 && !runID.isEmpty && width > 0 && height > 0 && width <= 512 && height <= 512
        && occupancy.count == width*height && occupancy.allSatisfy { (-1...100).contains($0) }
        && resolution.isFinite && resolution >= 0.01 && resolution <= 2 && originX.isFinite && originY.isFinite
        && age.isFinite && age >= 0 && age < 0.5
    }
    func value(x: Double, y: Double) -> Int? {
        guard usable, x.isFinite, y.isFinite else { return nil }
        let col = floor((x-originX)/resolution), row = floor((y-originY)/resolution)
        guard col >= 0, row >= 0, col < Double(width), row < Double(height) else { return nil }
        return occupancy[Int(row)*width+Int(col)]
    }
}

struct TargetSearchReport: Codable {
    var runID: String; var searchID: String; var targetClass: String; var reportID: String; var worldX: Double; var worldY: Double; var frameIDs: [UInt64]; var evidenceIDs: [String]
    enum CodingKeys: String, CodingKey { case runID = "run_id", searchID = "search_id", targetClass = "target_class", reportID = "report_id", worldX = "world_x", worldY = "world_y", frameIDs = "frame_ids", evidenceIDs = "evidence_ids" }
}
struct TargetSearchState: Codable {
    var runID: String; var searchID: String; var phase: String; var reason: String; var elapsed: Double
    var detectorAvailable: Bool; var token: String?; var result: String?; var report: TargetSearchReport?; var age: Double
    enum CodingKeys: String, CodingKey { case runID = "run_id", searchID = "search_id", phase, reason, elapsed = "elapsed_s", detectorAvailable = "detector_available", token, result, report, age }
    var fresh: Bool { age.isFinite && age >= 0 && age < 0.5 }
    var terminal: Bool { ["completed", "cancelled", "timed_out", "exhausted"].contains(phase) }
}
