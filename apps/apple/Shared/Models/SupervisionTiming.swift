import Foundation

/// High-level supervision tolerates forest link gaps; manual motion uses separate strict deadlines.
enum SupervisionTiming {
  static let staleAfter = 90.0
  static let acknowledgmentTimeout = 150.0
  static func usable(_ age: Double) -> Bool { age.isFinite && age >= 0 && age < staleAfter }
}
