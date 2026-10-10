import SwiftUI

@main struct ARGOSApp: App {
  @StateObject private var model = FleetModel()
  var body: some Scene {
    WindowGroup {
      FleetView().environmentObject(model)
        #if os(macOS)
          .frame(minWidth: 1180, minHeight: 800)
        #endif
    }
    #if os(macOS)
      .commands {
        CommandGroup(after: .appInfo) {
          Button("Disconnect") { Task { await model.disconnect() } }.keyboardShortcut(
            "d", modifiers: [.command, .shift])
        }
      }
    #endif
  }
}
