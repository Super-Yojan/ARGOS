import GameController
import SwiftUI

#if os(macOS)
  import AppKit
#endif

struct OccupancyFrame: Codable {
  var schemaVersion: Int
  var roverID: UInt64
  var runID: String
  var sequence: UInt64
  var width: Int
  var height: Int
  var resolution: Double
  var originX: Double
  var originY: Double
  var occupancy: [Int]
  var age: Double
  enum CodingKeys: String, CodingKey {
    case schemaVersion = "schema_version"
    case roverID = "rover_id"
    case runID = "run_id"
    case sequence, width, height, resolution
    case originX = "origin_x"
    case originY = "origin_y"
    case occupancy, age
  }
  var usable: Bool {
    schemaVersion == 1 && !runID.isEmpty && width > 0 && height > 0 && width <= 512 && height <= 512
      && occupancy.count == width * height && occupancy.allSatisfy { (-1...100).contains($0) }
      && resolution.isFinite && resolution >= 0.01 && resolution <= 2 && originX.isFinite
      && originY.isFinite
      && age.isFinite && age >= 0 && age < 0.5
  }
  func value(x: Double, y: Double) -> Int? {
    guard usable, x.isFinite, y.isFinite else { return nil }
    let col = floor((x - originX) / resolution)
    let row = floor((y - originY) / resolution)
    guard col >= 0, row >= 0, col < Double(width), row < Double(height) else { return nil }
    return occupancy[Int(row) * width + Int(col)]
  }
}
