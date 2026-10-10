import Foundation
import MapKit
import SwiftUI

enum DraftError: LocalizedError {
  case invalid
  var errorDescription: String? { "Enter two finite numeric coordinates." }
}
