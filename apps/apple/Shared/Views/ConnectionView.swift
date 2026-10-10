import SwiftUI

struct ConnectionView: View {
  @EnvironmentObject var model: FleetModel
  @Environment(\.dismiss) private var dismiss
  @AppStorage("endpoint") private var endpoint = "tcp/127.0.0.1:7448"
  @AppStorage("prefix") private var prefix = "terra/phone"
  @AppStorage("geographic") private var geographic = false
  @AppStorage("anchorLatitude") private var anchorLatitude = 38.8297
  @AppStorage("anchorLongitude") private var anchorLongitude = -77.3075
  var body: some View {
    NavigationStack {
      Form {
        Section("Connection profiles") {
          HStack {
            Button("Phone router") {
              #if os(macOS)
                endpoint = "tcp/127.0.0.1:7448"
              #else
                endpoint = "tcp/ROUTER_ADDRESS:7448"
              #endif
              prefix = "terra/phone"
              geographic = false
            }
            Button("Terra simulator") {
              endpoint = "tcp/127.0.0.1:7447"
              prefix = "terra/rover"
              geographic = false
            }
          }.disabled(model.busy)
          Text("Profiles fill in settings. Choose Connect to open the session.").font(.caption)
            .foregroundStyle(.secondary)
        }
        Section("Fleet connection") {
          TextField("Endpoint", text: $endpoint).accessibilityIdentifier("endpoint")
          TextField("Topic prefix", text: $prefix)
          #if os(macOS)
            Text(
              "For the phone router on this Mac, use localhost:7448 and the phone's matching topic prefix."
            ).font(.caption).foregroundStyle(.secondary)
          #else
            Text("Use the router Mac's Tailscale or LAN address. Localhost refers to this device.")
              .font(.caption).foregroundStyle(.secondary)
          #endif
        }
        Section("Positioning") {
          Text(
            "The phone automatically reports local or geographic positioning from tracking, GPS and heading quality."
          ).font(.caption)
        }
        Section {
          Button(model.busy ? "Connecting…" : "Connect") {
            Task {
              await model.connect(endpoint: endpoint, prefix: prefix)
              if model.snapshot.link != "disconnected" { dismiss() }
            }
          }.disabled(model.busy).accessibilityIdentifier("connect")
          if model.snapshot.link != "disconnected" {
            Button("Disconnect", role: .destructive) {
              Task {
                await model.disconnect()
                dismiss()
              }
            }.disabled(model.busy)
          }
          Text("Disconnecting ARGOS does not cancel a rover's latched waypoint.").font(.caption)
            .foregroundStyle(.secondary)
        }
      }.formStyle(.grouped).navigationTitle("Connection settings")
        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
    }
    #if os(macOS)
      .frame(width: 520, height: 620)
    #endif
  }
}
