import SwiftUI

struct ConnectionView: View {
  @EnvironmentObject var model: FleetModel
  @Environment(\.dismiss) private var dismiss
  @StateObject private var router = RouterController.shared
  @AppStorage("routerMode") private var routerMode = "remote"
  @AppStorage("routerTailscaleAddress") private var routerAddress = ""
  @State private var confirmRouterStop = false
  @AppStorage("routerSharing") private var routerSharing = "loopback"
  @StateObject private var discovery = RouterDiscovery()
  @AppStorage("endpoint") private var endpoint = "tcp/127.0.0.1:7448"
  @AppStorage("prefix") private var prefix = "terra/phone"
  @AppStorage("geographic") private var geographic = false
  @AppStorage("anchorLatitude") private var anchorLatitude = 38.8297
  @AppStorage("anchorLongitude") private var anchorLongitude = -77.3075
  var body: some View {
    NavigationStack {
      Form {
        Section("Router location") {
          Picker("Router", selection: $routerMode) {
            #if os(macOS)
            Text("Local on this Mac").tag("local")
            #endif
            Text("Remote host").tag("remote")
          }.disabled(model.busy || router.busy)
          #if os(macOS)
          if routerMode == "local" {
            Picker("Share router", selection: $routerSharing) {
              Text("Only this Mac").tag("loopback")
              Text("Local network (0.0.0.0)").tag("lan")
              Text("Tailscale network").tag("tailscale")
            }.disabled(router.running || router.busy)
            if routerSharing == "lan" {
              Text("Accepts connections on all network interfaces. Use on a trusted LAN; phones connect to this Mac's LAN address or discover ARGOS below.").font(.caption).foregroundStyle(.orange)
            }
            if routerSharing == "tailscale" {
            TextField("Mac Tailscale IPv4 address", text: $routerAddress)
              .disabled(router.running || router.busy)
            }
            Text("Local endpoint: tcp/127.0.0.1:7448. Phones connect to this Mac's Tailscale address on port 7448.").font(.caption)
            Label(router.running ? "Local router running" : "Local router stopped", systemImage: router.running ? "network" : "network.slash")
            if router.running {
              Button("Stop local router", role: .destructive) {
                if model.snapshot.link != "disconnected" { confirmRouterStop = true }
                else { Task { await router.stop() } }
              }.disabled(router.busy || model.busy)
            }
          }
          #endif
          if let error = router.error { Text(error).foregroundStyle(.orange) }
          DisclosureGroup("Router logs") {
            Text(router.logs.joined(separator: "\n").isEmpty ? "No local router events" : router.logs.joined(separator: "\n"))
              .font(.caption.monospaced()).textSelection(.enabled)
          }
        }
        Section("Fleet connection") {
          if routerMode == "remote" {
            Button("Find LAN routers") { discovery.find() }
            if !discovery.routers.isEmpty {
              Menu("Choose discovered router") {
                ForEach(discovery.routers, id: \.self) { address in
                  Button(address) { endpoint = address }
                }
              }
            }
            if discovery.searching { Text("Searching this LAN…").font(.caption) }
          }
          TextField("Remote endpoint", text: $endpoint).accessibilityIdentifier("endpoint").disabled(routerMode == "local")
          TextField("Topic prefix", text: $prefix)
          #if os(macOS)
            Text(
              "For the phone router on this Mac, use localhost:7448 and the phone's matching topic prefix."
            ).font(.caption).foregroundStyle(.secondary)
          #else
            Text("Use a discovered router or enter the router host's LAN/Tailscale address. Localhost refers to this device.")
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
              #if os(macOS)
              if routerMode == "local", routerSharing == "tailscale", routerAddress.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                router.error = "Enter this Mac's Tailscale IPv4 address before sharing."; return
              }
              if routerMode == "local", !(await router.start(address: routerSharing == "lan" ? "0.0.0.0" : routerSharing == "tailscale" ? routerAddress : "")) { return }
              #endif
              await model.connect(endpoint: routerMode == "local" ? "tcp/127.0.0.1:7448" : endpoint, prefix: prefix)
              if model.snapshot.link != "disconnected" { dismiss() }
            }
          }.disabled(model.busy).accessibilityIdentifier("connect")
          if model.snapshot.link != "disconnected" {
            Button("Disconnect", role: .destructive) {
              Task {
                await model.disconnect()
                dismiss()
              }
            }.disabled(model.busy && model.pendingWaypointID == nil)
          }
          Text("Disconnecting ARGOS does not cancel a rover's latched waypoint.").font(.caption)
            .foregroundStyle(.secondary)
        }
      }.formStyle(.grouped).navigationTitle("Connection settings")
        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
    }
    .onAppear {
      #if !os(macOS)
      routerMode = "remote"
      #endif
    }
    .confirmationDialog("Stop the local router and disconnect ARGOS? Other connected devices will lose this router.", isPresented: $confirmRouterStop, titleVisibility: .visible) {
      Button("Disconnect and stop router", role: .destructive) { Task { await model.disconnect(); await router.stop() } }
    }
    #if os(macOS)
      .frame(width: 520, height: 620)
    #endif
  }
}
