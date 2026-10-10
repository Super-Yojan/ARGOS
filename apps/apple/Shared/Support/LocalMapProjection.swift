import Foundation
import MapKit
import SwiftUI

enum LocalMapProjection {
  static func screen(
    x: Double, y: Double, size: CGSize, extent: Double, centerX: Double = 0, centerY: Double = 0
  ) -> CGPoint {
    let scale = min(size.width, size.height) / (2 * extent)
    return CGPoint(
      x: size.width / 2 - (y - centerY) * scale, y: size.height / 2 - (x - centerX) * scale)
  }
  static func world(
    _ point: CGPoint, size: CGSize, extent: Double, centerX: Double = 0, centerY: Double = 0
  ) -> (Double, Double) {
    let scale = min(size.width, size.height) / (2 * extent)
    return (
      centerX + (size.height / 2 - point.y) / scale, centerY + (size.width / 2 - point.x) / scale
    )
  }
}
