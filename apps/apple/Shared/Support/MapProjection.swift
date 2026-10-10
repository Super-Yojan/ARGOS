import Foundation
import MapKit
import SwiftUI

enum MapProjection {
  static let scale = 40_075_016.686 / 360.0
  static func coordinate(x: Double, y: Double, latitude: Double, longitude: Double)
    -> CLLocationCoordinate2D
  {
    CLLocationCoordinate2D(
      latitude: latitude + x / scale, longitude: longitude - y / (scale * cos(latitude * .pi / 180))
    )
  }
  static func local(_ coordinate: CLLocationCoordinate2D, latitude: Double, longitude: Double) -> (
    x: Double, y: Double
  ) {
    (
      (coordinate.latitude - latitude) * scale,
      -(coordinate.longitude - longitude) * scale * cos(latitude * .pi / 180)
    )
  }
}

// Attention is derived from observed transport state, never a fabricated health score.
