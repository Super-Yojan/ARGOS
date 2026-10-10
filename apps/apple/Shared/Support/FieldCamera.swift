import MapKit
import SwiftUI

struct FieldCamera {
  var x = 0.0
  var y = 0.0
  var extent = 30.0
  var dimensional = true
  func scale(_ size: CGSize) -> Double { max(1, min(size.width, size.height)) / (2 * extent) }
  var pitch: Double { dimensional ? 0.55 : 1 }
  func screen(x: Double, y: Double, z: Double = 0, size: CGSize) -> CGPoint {
    let s = scale(size)
    return CGPoint(
      x: size.width / 2 - (y - self.y) * s,
      y: size.height / 2 - (x - self.x) * s * pitch - z * s)
  }
  func world(_ point: CGPoint, size: CGSize) -> (x: Double, y: Double) {
    let s = scale(size)
    return (x + (size.height / 2 - point.y) / (s * pitch), y + (size.width / 2 - point.x) / s)
  }
}
