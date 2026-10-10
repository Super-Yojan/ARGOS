import Foundation
import MapKit
import SwiftUI

struct OccupancyState: Decodable {
  let grid: OccupancyGridState
  let age: Double
  var stale: Bool { age >= 2.5 }
}
