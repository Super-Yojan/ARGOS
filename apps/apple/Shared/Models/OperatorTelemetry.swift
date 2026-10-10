import GameController
import SwiftUI

#if os(macOS)
  import AppKit
#endif

struct OperatorTelemetry: Codable {
  var hardware: HardwareState?
  var cloud: PointCloudFrame?
  var authority: VehicleAuthority?
  var occupancy: OccupancyFrame?
  var search: TargetSearchState?
  var reports: [TargetSearchReport]?
}
