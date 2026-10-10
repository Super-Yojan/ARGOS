import Foundation
import MapKit
import SwiftUI

struct OperatorState: Decodable {
  var recording: String?
  var proposal_remaining: Double?
  var status: AuthorityState?
  var age: Double?
  var proposal: ProposalState?
  var mission: MissionState?
  var action_phase: String?
  var action_token: String?
  var canDrive: Bool {
    (age ?? .infinity) < 2.5 && status?.safety == "clear"
      && ["teleop", "assisted_teleop"].contains(status?.effective_level ?? "")
      && ["none", "accepted", "rejected"].contains(action_phase ?? "none")
  }
}
