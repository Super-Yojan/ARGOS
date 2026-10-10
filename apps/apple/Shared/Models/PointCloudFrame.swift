import GameController
import SwiftUI

#if os(macOS)
  import AppKit
#endif

struct PointCloudFrame: Codable {
  var version: Int
  var sequence: UInt64
  var frameID: String
  var source: String
  var age: Double
  var points: [[Double]]
  var usable: Bool {
    version == 1 && !frameID.isEmpty && age.isFinite && age >= 0 && age < 2.5
      && points.count <= 4096
      && points.allSatisfy { $0.count == 3 && $0.allSatisfy { $0.isFinite && abs($0) <= 20000 } }
  }
}
