import Foundation
import MapKit
import SwiftUI

struct OccupancyGridState: Decodable {
  let schema_version: Int
  let rover_id: UInt64
  let run_id: String
  let sequence: UInt64
  let width: Int
  let height: Int
  let resolution: Double
  let origin_x: Double
  let origin_y: Double
  let occupancy: [Int]
  var valid: Bool {
    schema_version == 1 && width > 0 && height > 0 && width <= 250000 && height <= 250000
      && width * height <= 250000 && occupancy.count == width * height && resolution.isFinite
      && resolution > 0 && origin_x.isFinite && origin_y.isFinite
      && occupancy.allSatisfy { (-1...100).contains($0) }
  }
}
