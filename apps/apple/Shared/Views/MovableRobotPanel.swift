import SwiftUI

/// Drag only the header, leaving scrolling, joystick and command gestures independent.
struct MovableRobotPanel<Content: View>: View {
  let title: String
  let containerSize: CGSize
  let panelSize: CGSize
  let initialCenter: CGPoint
  let close: () -> Void
  @ViewBuilder let content: () -> Content
  @State private var center: CGPoint?
  @GestureState private var translation = CGSize.zero

  private var restingCenter: CGPoint {
    Self.clamped(center ?? initialCenter, panel: panelSize, container: containerSize)
  }

  var body: some View {
    VStack(spacing: 0) {
      HStack(spacing: 8) {
        HStack {
          Image(systemName: "line.3.horizontal")
          Text(title).font(.headline)
          Spacer(minLength: 0)
        }
        .contentShape(Rectangle())
        .gesture(DragGesture(minimumDistance: 4)
          .updating($translation) { value, state, _ in state = value.translation }
          .onEnded { value in
            center = Self.clamped(CGPoint(x: restingCenter.x + value.translation.width,
                                          y: restingCenter.y + value.translation.height),
                                  panel: panelSize, container: containerSize)
          })
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Move robot controls")
        .accessibilityHint("Drag the header to reveal the map underneath")
        .accessibilityIdentifier("robot-panel-handle")
        .accessibilityAction(named: "Move left") { move(dx: -80, dy: 0) }
        .accessibilityAction(named: "Move right") { move(dx: 80, dy: 0) }
        .accessibilityAction(named: "Move up") { move(dx: 0, dy: -80) }
        .accessibilityAction(named: "Move down") { move(dx: 0, dy: 80) }
        Button(action: close) { Image(systemName: "xmark") }
          .buttonStyle(.plain).accessibilityLabel("Return to the place")
      }.padding(16)
      Divider()
      ScrollView { content() }
    }
    .frame(width: panelSize.width, height: panelSize.height)
    .background(Color(red: 0.025, green: 0.055, blue: 0.085).opacity(0.96), in: RoundedRectangle(cornerRadius: 12))
    .overlay { RoundedRectangle(cornerRadius: 12).stroke(Color.cyan.opacity(0.4), lineWidth: 1) }
    .position(Self.clamped(CGPoint(x: restingCenter.x + translation.width,
                                  y: restingCenter.y + translation.height),
                          panel: panelSize, container: containerSize))
    .onChange(of: containerSize) { _, _ in center = restingCenter }
    .onChange(of: panelSize) { _, _ in center = restingCenter }
  }

  private func move(dx: CGFloat, dy: CGFloat) {
    center = Self.clamped(CGPoint(x: restingCenter.x + dx, y: restingCenter.y + dy),
                          panel: panelSize, container: containerSize)
  }

  static func clamped(_ point: CGPoint, panel: CGSize, container: CGSize) -> CGPoint {
    let insetX = min(12, max(0, (container.width-panel.width)/2))
    let insetY = min(12, max(0, (container.height-panel.height)/2))
    let halfX = min(panel.width, container.width)/2
    let halfY = min(panel.height, container.height)/2
    return CGPoint(x: min(container.width-halfX-insetX, max(halfX+insetX, point.x)),
                   y: min(container.height-halfY-insetY, max(halfY+insetY, point.y)))
  }
}
