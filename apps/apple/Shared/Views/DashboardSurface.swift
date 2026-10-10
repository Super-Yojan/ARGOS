import MapKit
import SwiftUI

struct DashboardSurface: ViewModifier {
  func body(content: Content) -> some View {
    content.background(DashboardStyle.surface).overlay {
      Rectangle().stroke(DashboardStyle.line, lineWidth: 1)
    }
  }
}
