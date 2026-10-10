import SceneKit
import SwiftUI

// The optimized CAD mesh is shared by every node; telemetry remains in metres.
@MainActor final class RoverSceneController: ObservableObject {
  static let template: SCNNode? = {
    guard let url = Bundle.main.url(forResource: "Rover", withExtension: "scn"),
      let source = try? SCNScene(url: url, options: nil)
    else { return nil }
    let node = SCNNode()
    for child in source.rootNode.childNodes { node.addChildNode(child.clone()) }
    return node
  }()
  // Provisional 0.85 m (33.5 in) maximum footprint, bounded by the user's <35 in dimensions.
  static let physicalScale: Float = {
    guard let bounds = template?.boundingBox else { return 0.425 }
    return Float(0.85 / max(0.001, max(bounds.max.x - bounds.min.x, bounds.max.z - bounds.min.z)))
  }()
  let scene = SCNScene()
  let cameraNode = SCNNode()
  weak var view: SCNView?
  private let cloudNode = SCNNode()
  private var cloudFrame = ""
  private var cloudSequence: UInt64?
  private var cloudHistory: [[SCNVector3]] = []
  var robots: [UInt64: SCNNode] = [:]
  init() {
    cameraNode.camera = SCNCamera()
    cameraNode.camera?.usesOrthographicProjection = true
    cameraNode.camera?.zNear = 0.01
    cameraNode.camera?.zFar = 100000
    scene.rootNode.addChildNode(cameraNode)
    scene.rootNode.addChildNode(cloudNode)
    let ambient = SCNNode()
    ambient.light = SCNLight()
    ambient.light?.type = .ambient
    ambient.light?.intensity = 600
    scene.rootNode.addChildNode(ambient)
    let sun = SCNNode()
    sun.light = SCNLight()
    sun.light?.type = .directional
    sun.light?.intensity = 1200
    sun.eulerAngles = SCNVector3(-Float.pi / 3, -Float.pi / 4, 0)
    scene.rootNode.addChildNode(sun)
  }
  func update(rovers: [RoverView], camera: FieldCamera, size: CGSize, cloud: PointCloudFrame?) {
    SCNTransaction.begin()
    SCNTransaction.animationDuration = 0
    SCNTransaction.disableActions = true
    defer { SCNTransaction.commit() }
    cameraNode.camera?.orthographicScale =
      camera.extent * max(1, size.height) / max(1, min(size.width, size.height))
    // This pitch matches FieldCamera's ground-plane projection.
    let radius = max(100, camera.extent * 8)
    let pitch = camera.dimensional ? asin(0.55) : Double.pi / 2
    cameraNode.position = SCNVector3(
      -camera.y + cos(pitch) * radius * sin(camera.dimensional ? camera.azimuth : 0), sin(pitch) * radius, -camera.x + cos(pitch) * radius * cos(camera.dimensional ? camera.azimuth : 0))
    cameraNode.eulerAngles = SCNVector3(-pitch, camera.dimensional ? camera.azimuth : 0, 0)
    updateCloud(cloud)
    let present = Set(rovers.filter { $0.pose != nil }.map(\.id))
    for id in Array(robots.keys) where !present.contains(id) {
      robots.removeValue(forKey: id)?.removeFromParentNode()
    }
    for rover in rovers {
      guard let pose = rover.pose else { continue }
      let node: SCNNode
      if let existing = robots[rover.id] {
        node = existing
      } else {
        guard let mesh = Self.template?.clone() else { continue }
        node = mesh
        node.name = "rover:\(rover.id)"
        robots[rover.id] = node
        scene.rootNode.addChildNode(node)
      }
      node.position = SCNVector3(-pose.y, 0, -pose.x)
      node.eulerAngles = SCNVector3(0, Float(pose.yaw), 0)
      // Geometry stays at physical scale at every zoom; callouts remain tappable.
      let displayScale = Self.physicalScale
      node.scale = SCNVector3(displayScale, displayScale, displayScale)
      node.opacity = (rover.poseAge ?? 99) >= SupervisionTiming.staleAfter ? 0.45 : 1
    }
  }
  private func updateCloud(_ cloud: PointCloudFrame?) {
    guard let cloud, cloud.usable else {
      cloudNode.geometry = nil
      cloudHistory = []
      cloudSequence = nil
      cloudFrame = ""
      return
    }
    if cloud.frameID != cloudFrame {
      cloudHistory = []
      cloudSequence = nil
      cloudFrame = cloud.frameID
    }
    guard cloud.sequence != cloudSequence else { return }
    cloudSequence = cloud.sequence
    cloudHistory.append(cloud.points.map { SCNVector3(-$0[1], $0[2], -$0[0]) })
    if cloudHistory.count > 8 { cloudHistory.removeFirst() }
    let points = cloudHistory.flatMap { $0 }
    guard !points.isEmpty else {
      cloudNode.geometry = nil
      return
    }
    let source = SCNGeometrySource(vertices: points)
    let element = SCNGeometryElement(indices: Array(0..<Int32(points.count)), primitiveType: .point)
    element.pointSize = 3
    element.minimumPointScreenSpaceRadius = 1
    element.maximumPointScreenSpaceRadius = 3
    let geometry = SCNGeometry(sources: [source], elements: [element])
    let material = SCNMaterial()
    material.lightingModel = .constant
    #if os(macOS)
      material.diffuse.contents = NSColor.cyan
    #else
      material.diffuse.contents = UIColor.cyan
    #endif
    geometry.materials = [material]
    cloudNode.geometry = geometry
  }
  func rover(at point: CGPoint) -> UInt64? {
    guard let view else { return nil }
    var location = point
    #if os(macOS)
      if !view.isFlipped { location.y = view.bounds.height - point.y }
    #endif
    for hit in view.hitTest(location, options: [:]) {
      var node: SCNNode? = hit.node
      while let current = node {
        if let name = current.name, name.hasPrefix("rover:"), let id = UInt64(name.dropFirst(6)) {
          return id
        }
        node = current.parent
      }
    }
    return nil
  }
  func makeView() -> SCNView {
    let view = PassThroughRoverView()
    view.scene = scene
    view.pointOfView = cameraNode
    view.allowsCameraControl = false
    view.backgroundColor = .clear
    view.antialiasingMode = .multisampling4X
    view.autoenablesDefaultLighting = false
    view.preferredFramesPerSecond = 30
    #if !os(macOS)
      view.isUserInteractionEnabled = false
    #endif
    self.view = view
    return view
  }
}
