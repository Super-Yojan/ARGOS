import SwiftUI
struct FleetView: View {
    @EnvironmentObject var model: FleetModel
    @Environment(\.scenePhase) private var scenePhase
    @State private var settings=false
    @AppStorage("endpoint") private var endpoint="tcp/127.0.0.1:7447"
    @AppStorage("prefix") private var prefix="terra/rover"
    var body: some View {
        NavigationSplitView {
            List(selection:$model.selected) {
                Section {
                    HStack {Circle().fill(model.snapshot.link=="healthy" ? Color.green:Color.secondary).frame(width:8,height:8);Text(model.snapshot.link.capitalized).font(.subheadline);Spacer();if model.busy {ProgressView().controlSize(.small)}}
                    .accessibilityElement(children:.combine)
                } header: {Text("Fleet connection")}
                Section("Mission actions") {
                    Button("Take over fleet") {Task {await model.fleetAction(stop:false)}}
                    Button("Emergency stop fleet",role:.destructive) {Task {await model.fleetAction(stop:true)}}
                    ForEach(model.snapshot.rovers,id:\.id) {rover in
                        if let phase=model.state(rover.id)?.action_phase,phase != "none" {Text("Rover \(rover.id): \(phase)").font(.caption)}
                    }
                    Button("Export session log") {Task {await model.exportLog()}}
                    if let url=model.exportedLog {ShareLink("Save or share session log",item:url)}
                }
                Section("Rovers · \(model.snapshot.rovers.count)") {
                    ForEach(model.snapshot.rovers,id:\.id) {rover in
                        VStack(alignment:.leading,spacing:5) {
                            HStack {Image(systemName:"location.north.circle");Text("Rover \(rover.id)").font(.headline);Spacer()}
                            Text("\(rover.membership.capitalized) · \(rover.goal?.state.capitalized ?? "Goal unknown")").font(.caption).foregroundStyle(.secondary)
                        }.padding(.vertical,5).tag(rover.id)
                    }
                }
                if model.snapshot.rovers.isEmpty {Text("Connect to Terra to discover your fleet.").foregroundStyle(.secondary).padding(.vertical)}
            }
            .navigationTitle("ARGOS")
            .toolbar {ToolbarItem {Button {settings=true} label:{Label("Connection",systemImage:"network")}.accessibilityIdentifier("connection")}}
        } detail: {
            if let rover=model.snapshot.rovers.first(where:{$0.id==model.selected}) {RoverDetailView(rover:rover)}
            else {ContentUnavailableView("Fleet overview",systemImage:"map",description:Text("Connect to Terra, then select a rover to set a waypoint.")).toolbar {ToolbarItem {Button("Connect") {settings=true}}}}
        }
        .sheet(isPresented:$settings) {ConnectionView()}
        .alert("Operator notice",isPresented:Binding(get:{model.error != nil},set:{if !$0 {model.error=nil}})) {Button("OK") {model.error=nil}} message:{Text(model.error ?? "")}
        .onChange(of:scenePhase) {_,phase in if phase == .active {model.startObservation()}else {model.clearInput();model.stopObservation()}}
        .task {if ProcessInfo.processInfo.arguments.contains("--connect") {await model.connect(endpoint:endpoint,prefix:prefix)}}
    }
}
