import GameController
import SwiftUI

#if os(macOS)
  import AppKit
#endif

struct TargetSearchReport: Codable {
  var runID: String
  var searchID: String
  var targetClass: String
  var reportID: String
  var worldX: Double
  var worldY: Double
  var frameIDs: [UInt64]
  var evidenceIDs: [String]
  enum CodingKeys: String, CodingKey {
    case runID = "run_id"
    case searchID = "search_id"
    case targetClass = "target_class"
    case reportID = "report_id"
    case worldX = "world_x"
    case worldY = "world_y"
    case frameIDs = "frame_ids"
    case evidenceIDs = "evidence_ids"
  }
}
