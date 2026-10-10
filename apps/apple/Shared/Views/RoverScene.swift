import SceneKit
import SwiftUI

#if os(macOS)
  final class PassThroughRoverView: SCNView {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
  }
  struct RoverScene: NSViewRepresentable {
    let controller: RoverSceneController
    let rovers: [RoverView]
    let camera: FieldCamera
    let size: CGSize
    let cloud: PointCloudFrame?
    func makeCoordinator() -> RoverSceneController { controller }
    func makeNSView(context: Context) -> SCNView { context.coordinator.makeView() }
    func updateNSView(_ view: SCNView, context: Context) {
      context.coordinator.update(rovers: rovers, camera: camera, size: size, cloud: cloud)
    }
  }
#else
  final class PassThroughRoverView: SCNView {}
  struct RoverScene: UIViewRepresentable {
    let controller: RoverSceneController
    let rovers: [RoverView]
    let camera: FieldCamera
    let size: CGSize
    let cloud: PointCloudFrame?
    func makeCoordinator() -> RoverSceneController { controller }
    func makeUIView(context: Context) -> SCNView { context.coordinator.makeView() }
    func updateUIView(_ view: SCNView, context: Context) {
      context.coordinator.update(rovers: rovers, camera: camera, size: size, cloud: cloud)
    }
  }
#endif
