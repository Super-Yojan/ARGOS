import SwiftUI

struct RoverJoystick: View {
  @ObservedObject var drive: RoverDriveController
  var body: some View {
    ZStack {
      Circle().stroke(.secondary, lineWidth: 1)
      Image(systemName: "plus").foregroundStyle(.secondary)
      Circle().fill(.primary).frame(width: 24, height: 24).offset(
        x: drive.stick.x * 36, y: drive.stick.y * 36)
    }.frame(width: 110, height: 110).contentShape(Circle())
      .gesture(
        DragGesture(minimumDistance: 0).onChanged { value in
          let x = (value.location.x - 55) / 42
          let y = (value.location.y - 55) / 42
          let length = max(1, hypot(x, y))
          drive.joystick(CGPoint(x: x / length, y: y / length))
        }.onEnded { _ in drive.joystick(nil) }
      )
      .opacity(drive.enabled ? 1 : 0.3).allowsHitTesting(drive.enabled)
      .accessibilityLabel("Driving joystick; hold and drag to move, release to stop")
      .accessibilityIdentifier("drive-joystick")
  }
}
