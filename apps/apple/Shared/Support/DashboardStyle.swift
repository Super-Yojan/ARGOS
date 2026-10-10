import MapKit
import SwiftUI

enum DashboardStyle {
  static let background = Color(white: 0.92)
  static let surface = Color.white
  static let accent = Color(white: 0.12)
  static let text = Color(white: 0.12)
  static let muted = Color(white: 0.4)
  static let line = Color.black.opacity(0.12)
  static func vehicle(_ id: UInt64) -> String { "T-\(id)" }
}
