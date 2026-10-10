import MapKit
import SwiftUI

enum FieldMarkerPlacement {
  static func locations(desired: [(UInt64, CGPoint)], size: CGSize) -> [UInt64: CGPoint] {
    var slots: [CGPoint] = []
    let width = max(160, size.width)
    let height = max(160, size.height)
    let columns = max(1, Int(width / 170))
    let rows = max(1, Int((height - 140) / 65))
    for column in 0..<columns {
      let x = (Double(column) + 0.5) * width / Double(columns)
      slots.append(CGPoint(x: x, y: 75))
      slots.append(CGPoint(x: x, y: height - 75))
    }
    for row in 0..<rows {
      let y = 110 + (Double(row) + 0.5) * max(1, height - 220) / Double(rows)
      slots.append(CGPoint(x: 80, y: y))
      slots.append(CGPoint(x: width - 80, y: y))
    }
    var result: [UInt64: CGPoint] = [:]
    for (id, target) in desired.sorted(by: { $0.0 < $1.0 }) {
      guard
        let index = slots.indices.min(by: {
          hypot(slots[$0].x - target.x, slots[$0].y - target.y)
            < hypot(slots[$1].x - target.x, slots[$1].y - target.y)
        })
      else { break }
      result[id] = slots.remove(at: index)
    }
    return result
  }
}
