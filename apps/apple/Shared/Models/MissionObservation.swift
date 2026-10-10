import Foundation
import MapKit
import SwiftUI

struct MissionObservation: Decodable {
  var survivor_id: UInt64
  var x: Double
  var y: Double
  var rover_id: UInt64
  var confirmed: Bool
}
