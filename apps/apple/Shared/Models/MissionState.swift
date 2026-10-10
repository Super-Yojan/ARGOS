import Foundation
import MapKit
import SwiftUI

struct MissionState: Decodable {
  var mission_id: String
  var objective: String
  var phase: String
  var confirmed: Int
  var required: Int
  var remaining_seconds: Double
  var observations: [MissionObservation]
}
