import MapKit
import SwiftUI

struct FieldCamera {
  var x = 0.0
  var y = 0.0
  var extent = 30.0
  var dimensional = true
  var azimuth = 0.0
  func scale(_ size: CGSize) -> Double { max(1, min(size.width, size.height)) / (2 * extent) }
  var pitch: Double { dimensional ? 0.55 : 1 }
  func screen(x: Double, y: Double, z: Double = 0, size: CGSize) -> CGPoint {
    let s = scale(size)
    let a = dimensional ? azimuth : 0
    let u = (x - self.x) * cos(a) + (y - self.y) * sin(a)
    let v = (y - self.y) * cos(a) - (x - self.x) * sin(a)
    return CGPoint(x: size.width / 2 - v * s, y: size.height / 2 - u * s * pitch - z * s)
  }
  func world(_ point: CGPoint, size: CGSize) -> (x: Double, y: Double) {
    let s = scale(size)
    let a = dimensional ? azimuth : 0
    let u = (size.height / 2 - point.y) / (s * pitch)
    let v = (size.width / 2 - point.x) / s
    return (x + u * cos(a) - v * sin(a), y + u * sin(a) + v * cos(a))
  }
}
