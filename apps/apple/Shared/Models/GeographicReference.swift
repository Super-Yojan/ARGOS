import Foundation
import MapKit
import SwiftUI

struct GeographicReference: Codable {
  var version: Int
  var frameID: String
  var mode: String
  var reason: String
  var tracking: String
  var age: Double
  var gpsAccuracy: Double?
  var headingAccuracy: Double?
  var originLatitude: Double?
  var originLongitude: Double?
  var anchorX: Double?
  var anchorY: Double?
  var rotation: Double?
  var usable: Bool {
    version == 1 && !frameID.isEmpty && mode == "geographic" && tracking == "normal" && age.isFinite
      && age >= 0 && age < 2.5
      && gpsAccuracy.map { $0.isFinite && $0 >= 0 && $0 <= 10 } == true
      && headingAccuracy.map { $0.isFinite && $0 >= 0 && $0 <= 15 } == true
      && originLatitude.map { $0.isFinite && abs($0) <= 85 } == true
      && originLongitude.map { $0.isFinite && abs($0) <= 180 } == true
      && anchorX?.isFinite == true && anchorY?.isFinite == true && rotation?.isFinite == true
  }
  var origin: CLLocationCoordinate2D? {
    guard usable else { return nil }
    return CLLocationCoordinate2D(latitude: originLatitude!, longitude: originLongitude!)
  }
  func scene(x: Double, y: Double, origin: CLLocationCoordinate2D) -> (x: Double, y: Double) {
    let dx = x - anchorX!
    let dy = y - anchorY!
    let r = rotation!
    let coordinate = MapProjection.coordinate(
      x: cos(r) * dx - sin(r) * dy, y: sin(r) * dx + cos(r) * dy, latitude: originLatitude!,
      longitude: originLongitude!)
    return MapProjection.local(coordinate, latitude: origin.latitude, longitude: origin.longitude)
  }
  func local(x: Double, y: Double, origin: CLLocationCoordinate2D) -> (x: Double, y: Double) {
    let coordinate = MapProjection.coordinate(
      x: x, y: y, latitude: origin.latitude, longitude: origin.longitude)
    let p = MapProjection.local(coordinate, latitude: originLatitude!, longitude: originLongitude!)
    let r = rotation!
    return (anchorX! + cos(r) * p.x + sin(r) * p.y, anchorY! - sin(r) * p.x + cos(r) * p.y)
  }
}
