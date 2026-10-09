import SwiftUI

struct RoverDetailView: View {
    @EnvironmentObject var model: FleetModel
    let rover: RoverView
    var dashboard = false
    @AppStorage("geographic") private var geographic = false
    @AppStorage("anchorLatitude") private var anchorLatitude = 38.8297
    @AppStorage("anchorLongitude") private var anchorLongitude = -77.3075
    @State private var acknowledgedReason: String?
    @State private var first = ""
    @State private var second = ""
    private var ready: Bool { rover.membership == "online" && model.snapshot.link == "healthy" && !model.busy }
    var body: some View {
        Group {
            if dashboard {
                HStack(alignment: .top, spacing: 16) {
                    mapPanel.frame(maxWidth: .infinity).modifier(DashboardSurface())
                    ScrollView { inspector.padding(18) }.frame(width: 300).modifier(DashboardSurface())
                }
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 20) {
                        mapPanel.frame(height: 340).modifier(DashboardSurface())
                        inspector
                    }.padding(20)
                }
            }
        }
        .navigationTitle("Rover \(rover.id)")
        .onChange(of: rover.id) { _, _ in first = ""; second = ""; acknowledgedReason = nil }
        .onChange(of: geographic) { _, _ in first = ""; second = "" }
    }
    private func stepVehicle(_ direction: Int) {
        let vehicles = model.snapshot.rovers
        guard !vehicles.isEmpty, let index = vehicles.firstIndex(where: { $0.id == rover.id }) else { return }
        model.selected = vehicles[(index + direction + vehicles.count) % vehicles.count].id
    }
    private var mapPanel: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Text("Live map").font(.headline)
                Label(rover.pose == nil ? "No pose" : (rover.poseAge ?? 99) >= 2.5 ? "Stale pose" : "Live", systemImage: "circle.fill")
                    .font(.caption).foregroundStyle(rover.pose == nil || (rover.poseAge ?? 99) >= 2.5 ? Color.orange : Color.green)
                Spacer(minLength: 8)
                Picker("Map coordinates", selection: $geographic) {
                    Text("Local").tag(false); Text("Geographic").tag(true)
                }.pickerStyle(.segmented).labelsHidden().frame(width: 185).accessibilityLabel("Map coordinate mode")
            }.padding(16)
            Group {
                if geographic {
                    GeographicMap(rovers: model.snapshot.rovers, selected: rover.id, latitude: anchorLatitude, longitude: anchorLongitude, draft: coordinateDraft, select: { model.selected = $0 }) { lat, lon in
                        first = String(format: "%.7f", lat); second = String(format: "%.7f", lon)
                    }
                } else {
                    LocalMap(rovers: model.snapshot.rovers, selected: rover.id, draft: localDraft, select: { model.selected = $0 }) { x, y in
                        first = String(format: "%.2f", x); second = String(format: "%.2f", y)
                    }
                }
            }.clipShape(RoundedRectangle(cornerRadius: 5)).padding(.horizontal, 10)
            HStack {
                Label(geographic ? "Configured geographic anchor" : "Observed poses · local coordinates", systemImage: "location")
                Spacer()
                Text("Select a vehicle marker · click empty space to draft")
            }.font(.caption).foregroundStyle(DashboardStyle.muted).padding(12)
        }
    }
    private var inspector: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("Selected vehicle").font(.system(size: 13, weight: .semibold)).lineLimit(1).layoutPriority(1)
                Spacer()
                Button { stepVehicle(-1) } label: { Image(systemName: "chevron.left") }.accessibilityLabel("Previous vehicle")
                Button { stepVehicle(1) } label: { Image(systemName: "chevron.right") }.accessibilityLabel("Next vehicle")
            }
            Divider()
            HStack(spacing: 12) {
                Image(systemName: "car.side").font(.system(size: 31)).foregroundStyle(DashboardStyle.muted)
                VStack(alignment: .leading, spacing: 5) {
                    Text(DashboardStyle.vehicle(rover.id)).font(.title2.weight(.semibold))
                    Text("Terra rover \(rover.id)").font(.caption).foregroundStyle(DashboardStyle.muted)
                }
                Spacer()
            }
            HStack {
                Label(rover.membership.capitalized, systemImage: "circle.fill").font(.caption)
                    .foregroundStyle(rover.membership == "online" ? Color.green : Color.orange)
                Text("Ground").font(.caption).padding(.horizontal, 8).padding(.vertical, 4).background(Color.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 4))
                Spacer()
                Text("Autonomy not reported").font(.caption).foregroundStyle(DashboardStyle.muted)
            }
            if let notice = model.snapshot.notice {
                Label(notice, systemImage: "info.circle").font(.caption).foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }
            telemetry
            Divider()
            waypointEditor
            if rover.commandPhase != "none" {
                Label(commandLabel, systemImage: rover.commandPhase == "unconfirmed" || rover.commandPhase == "failed" ? "exclamationmark.triangle" : "arrow.triangle.2.circlepath")
                    .font(.subheadline).fixedSize(horizontal: false, vertical: true).accessibilityIdentifier("command-phase")
            }
            if let token = rover.commandToken ?? rover.goal?.token {
                DisclosureGroup("Command receipt") { Text(token).font(.caption.monospaced()).textSelection(.enabled).padding(.top, 6) }
                    .font(.caption).foregroundStyle(DashboardStyle.muted)
            }
            if let reason = DashboardPresentation.attentionReason(rover) {
                VStack(alignment: .leading, spacing: 8) {
                    Label(reason, systemImage: "exclamationmark.triangle").font(.caption).foregroundStyle(.orange)
                    if acknowledgedReason == reason { Text("Acknowledged").font(.caption).foregroundStyle(DashboardStyle.muted) }
                    else { Button("Acknowledge") { acknowledgedReason = reason }.buttonStyle(.bordered).controlSize(.small) }
                }
            }
            Text("Cancellation clears the waypoint. It does not issue a physical emergency stop.")
                .font(.caption).foregroundStyle(DashboardStyle.muted).fixedSize(horizontal: false, vertical: true)
        }
    }
    private var telemetry: some View {
        VStack(spacing: 0) {
            telemetryRow("Position", symbol: "location", value: rover.pose.map { String(format: "%.2f, %.2f m", $0.x, $0.y) } ?? "Unknown")
            telemetryRow("Pose age", symbol: "clock", value: freshness(rover.poseAge))
            telemetryRow("Heading", symbol: "location.north", value: rover.pose.map { String(format: "%.0f° in local frame", $0.yaw * 180 / .pi) } ?? "Unknown")
            telemetryRow("Current task", symbol: "scope", value: rover.goal?.state == "active" ? "Navigating to waypoint" : rover.goal?.state == "arrived" ? "Waypoint reached" : rover.goal?.state == "idle" ? "Idle" : "Unknown")
            telemetryRow("Goal status", symbol: "flag", value: rover.goal?.state.capitalized ?? "Unknown")
            if let goal = rover.goal, goal.state != "idle" { telemetryRow("Remaining", symbol: "arrow.up.right", value: String(format: "%.2f m", goal.distance)) }
            telemetryRow("Status age", symbol: "clock", value: freshness(rover.goalAge))
            DisclosureGroup("Additional telemetry") {
                VStack(spacing: 0) {
                    telemetryRow("Battery", symbol: "battery.100", value: "Not reported")
                    telemetryRow("Speed", symbol: "speedometer", value: "Not reported")
                    telemetryRow("Camera", symbol: "video", value: "Not available")
                }
            }.font(.caption).foregroundStyle(DashboardStyle.muted).padding(.top, 12)
        }
    }
    private func telemetryRow(_ title: String, symbol: String, value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Label(title, systemImage: symbol).foregroundStyle(DashboardStyle.muted)
            Spacer(minLength: 8)
            Text(value).multilineTextAlignment(.trailing).monospacedDigit()
        }.font(.system(size: 12)).padding(.vertical, 9)
            .overlay(alignment: .bottom) { Rectangle().fill(DashboardStyle.line).frame(height: 1) }
    }
    private var waypointEditor: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Assign waypoint").font(.headline)
            Text("Choose a point on the map or enter a target.").font(.caption).foregroundStyle(DashboardStyle.muted)
            HStack {
                coordinateField(geographic ? "Latitude" : "x · metres", text: $first, identifier: "target-first")
                coordinateField(geographic ? "Longitude" : "y · metres", text: $second, identifier: "target-second")
            }
            HStack(spacing: 10) {
                Button("Send waypoint") {
                    do {
                        let waypoint = try WaypointDraft.make(first: first, second: second, geographic: geographic)
                        Task { await model.send(id: rover.id, waypoint: waypoint) }
                    } catch { model.error = error.localizedDescription }
                }.buttonStyle(.borderedProminent).disabled(!ready || (geographic ? coordinateDraft == nil : localDraft == nil) || ["pending", "cancelling"].contains(rover.commandPhase)).accessibilityIdentifier("send-waypoint")
                Button("Cancel goal", role: .destructive) { Task { await model.cancel(id: rover.id) } }
                    .buttonStyle(.bordered).disabled(!ready || rover.commandPhase == "cancelling").accessibilityIdentifier("cancel-goal")
            }.controlSize(.regular)
        }
    }
    private func coordinateField(_ title: String, text: Binding<String>, identifier: String) -> some View {
        VStack(alignment: .leading, spacing: 6) { Text(title).font(.caption).foregroundStyle(DashboardStyle.muted); TextField("", text: text).textFieldStyle(.roundedBorder).accessibilityLabel(title).accessibilityIdentifier(identifier) }
    }
    private var localDraft: (Double, Double)? {
        guard let a = Double(first.trimmingCharacters(in: .whitespaces)), let b = Double(second.trimmingCharacters(in: .whitespaces)), a.isFinite, b.isFinite else { return nil }
        return (a, b)
    }
    private var coordinateDraft: (Double, Double)? { guard let point = localDraft, abs(point.0) <= 85, abs(point.1) <= 180 else { return nil }; return point }
    private func freshness(_ age: Double?) -> String {
        guard let age else { return "Unknown" }
        return String(format: age >= 2.5 ? "Stale · %.1f s" : "%.1f s", age)
    }
    private var commandLabel: String {
        switch rover.commandPhase {
        case "pending": return "Sent · waiting for Terra acceptance"
        case "active": return "Waypoint accepted · driving"
        case "arrived": return "Arrived at waypoint"
        case "cancelling": return "Cancel sent · waiting for idle"
        case "cancelled": return "Goal cancelled · teleop released"
        case "unconfirmed": return "No acknowledgement · check Terra before sending again"
        case "superseded": return "Goal changed or cleared by Terra"
        case "failed": return "Publish failed"
        default: return rover.commandPhase.capitalized
        }
    }
}
