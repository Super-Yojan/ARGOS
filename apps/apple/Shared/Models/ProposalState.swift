import Foundation
import MapKit
import SwiftUI

struct ProposalState: Decodable {
  var run_id: String
  var proposal_id: UInt64
  var x: Double
  var y: Double
  var expires_at: Double
  var reason: String
}
