import Foundation
import MapKit
import SwiftUI

enum DashboardPresentation {
  static func attentionReason(_ rover: RoverView) -> String? {
    if rover.commandPhase == "failed" { return "Command failed" }
    if rover.commandPhase == "unconfirmed" { return "Command unconfirmed" }
    if rover.membership != "online" { return "Vehicle \(rover.membership)" }
    guard let poseAge = rover.poseAge else { return "Position unavailable" }
    if poseAge >= SupervisionTiming.staleAfter { return "Position is stale" }
    if rover.goal != nil {
      guard let goalAge = rover.goalAge else { return "Goal status unavailable" }
      if goalAge >= SupervisionTiming.staleAfter { return "Goal status is stale" }
    }
    return nil
  }
  static func attentionPriority(_ rover: RoverView) -> Int {
    if rover.commandPhase == "failed" { return 0 }
    if rover.commandPhase == "unconfirmed" { return 1 }
    if rover.membership != "online" { return 2 }
    if rover.poseAge == nil { return 3 }
    if (rover.poseAge ?? 0) >= SupervisionTiming.staleAfter { return 4 }
    return 5
  }
}

// Display alignment is separate from the rover's unchanged control frame.
