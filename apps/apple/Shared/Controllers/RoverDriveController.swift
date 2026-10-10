import GameController
import SwiftUI

#if os(macOS)
  import AppKit
#endif

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
        keyboardMonitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .keyUp]) {
          [weak self] event in
          guard let self, self.enabled else { return event }
          let names: [UInt16: String] = [
            13: "w", 1: "s", 0: "a", 2: "d", 49: "space", 126: "up", 125: "down", 123: "left",
            124: "right", 53: "escape",
          ]
          guard let key = names[event.keyCode] else { return event }
          if key == "escape"
            || event.modifierFlags.intersection([.command, .control, .option]).isEmpty == false
          {
            self.stop()
            return event
          }
          if event.type == .keyDown { self.keys.insert(key) } else { self.keys.remove(key) }
          return nil
        }
        focusObserver = NotificationCenter.default.addObserver(
          forName: NSWindow.didResignKeyNotification, object: nil, queue: .main
        ) { [weak self] _ in
          Task { @MainActor in self?.stop() }
        }
      }
    #endif
  }
  func start(id: UInt64) async {
    stop()
    guard let model, model.driveAvailable(id), model.motorsArmed(id) else {
      status = "Fresh rover authority required"
      return
    }
    let generation = self.generation
    roverID = id
    session = UUID().uuidString
    sequence = 0
    keys.removeAll()
    joystickHeld = false
    stick = .zero
    status = "Requesting takeover…"
    active = true
    requesting = true
    let level =
      model.authorities[id]?.requestedLevel == "assisted_teleop" ? "assisted_teleop" : "teleop"
    let receipt = await model.selectAutonomy(id: id, level: level)
    guard self.generation == generation, active, roverID == id else { return }
    guard let token = receipt else {
      active = false
      requesting = false
      status = "Takeover not confirmed"
      return
    }
    self.token = token
    task = Task { [weak self] in
      guard let self else { return }
      let deadline = Date().addingTimeInterval(3)
      while !Task.isCancelled && self.active && self.generation == generation {
        guard let authority = model.authorities[id], model.driveAvailable(id) else {
          self.stop(reason: "Driving stopped · telemetry unavailable")
          return
        }
        if authority.acceptsDrive(token: token) { break }
        if Date() >= deadline {
          self.stop(reason: "Takeover not acknowledged")
          return
        }
        try? await Task.sleep(for: .milliseconds(100))
      }
      guard !Task.isCancelled, self.active, self.generation == generation else { return }
      self.requesting = false
      self.enabled = true
      self.status = "Hold WASD / arrows, shoulder + gamepad, or drag joystick"
      while !Task.isCancelled && self.active && self.generation == generation {
        guard let authority = model.authorities[id], authority.acceptsDrive(token: token),
          model.driveAvailable(id)
        else {
          self.stop(reason: "Driving stopped · authority or tracking changed")
          return
        }
        self.sequence += 1
        let vector = self.input()
        do {
          try await model.drive(
            id: id, vector: vector, authority: authority, session: self.session,
            sequence: self.sequence)
        } catch {
          if self.generation == generation {
            self.stop(reason: "Driving stopped · \(error.localizedDescription)")
          }
          return
        }
        try? await Task.sleep(for: .milliseconds(100))
      }
    }
  }
  private func input() -> DriveVector {
    let pad = GCController.controllers().first { $0.extendedGamepad != nil }
    if gamepadName != pad?.vendorName { gamepadName = pad?.vendorName }
    #if !os(macOS)
      if let keyboard = GCKeyboard.coalesced?.keyboardInput {
        let map: [(GCKeyCode, String)] = [
          (.keyW, "w"), (.keyS, "s"), (.keyA, "a"), (.keyD, "d"), (.spacebar, "space"),
          (.upArrow, "up"), (.downArrow, "down"), (.leftArrow, "left"), (.rightArrow, "right"),
        ]
        keys = Set(map.filter { keyboard.button(forKeyCode: $0.0)?.isPressed == true }.map { $0.1 })
      } else {
        keys.removeAll()
      }
    #endif
    if joystickHeld { return .axes(forward: -stick.y, turn: -stick.x) }
    if !keys.intersection(["w", "a", "s", "d", "up", "down", "left", "right"]).isEmpty {
      return .keyboard(keys)
    }
    if let pad = pad?.extendedGamepad, pad.leftShoulder.isPressed {
      return .axes(
        forward: Double(pad.leftThumbstick.yAxis.value),
        turn: -Double(pad.leftThumbstick.xAxis.value))
    }
    return .zero
  }
  func joystick(_ point: CGPoint?) {
    guard enabled else { return }
    joystickHeld = point != nil
    stick = point ?? .zero
  }
  func stop(reason: String = "Driving off") {
    generation = UUID()
    active = false
    enabled = false
    requesting = false
    task?.cancel()
    task = nil
    keys.removeAll()
    joystickHeld = false
    stick = .zero
    status = reason
    if let model, let id = roverID, let authority = model.authorities[id] {
      sequence += 1
      let sequence = sequence
      let session = session
      Task {
        try? await model.drive(
          id: id, vector: .zero, authority: authority, session: session, sequence: sequence)
      }
    }
    roverID = nil
    token = nil
  }
  func detach() {
    stop()
    #if os(macOS)
      if let keyboardMonitor { NSEvent.removeMonitor(keyboardMonitor) }
      keyboardMonitor = nil
      if let focusObserver { NotificationCenter.default.removeObserver(focusObserver) }
      focusObserver = nil
    #endif
  }
}
