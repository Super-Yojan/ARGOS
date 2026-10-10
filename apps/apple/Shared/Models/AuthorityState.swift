import Foundation
import MapKit
import SwiftUI

struct AuthorityState: Decodable {
  var run_id: String?
  var assigned_level: String?
  var requested_level: String
  var effective_level: String?
  var active_source: String
  var safety: String
  var reason: String
  var revision: UInt64
  var supported_levels: [String]
  var paused: Bool
  var request_reason: String?
}
