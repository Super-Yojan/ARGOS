import SwiftUI
struct RoverDetailView:View {
    @EnvironmentObject var model:FleetModel
    let rover:RoverView
    @AppStorage("geographic") private var geographic=false
    @AppStorage("anchorLatitude") private var anchorLatitude=38.8297
    @AppStorage("anchorLongitude") private var anchorLongitude = -77.3075
    @State private var first=""
    @State private var second=""
    private var ready:Bool {rover.membership=="online" && model.snapshot.link=="healthy" && !model.busy}
    var body:some View {
        ScrollView {
            VStack(alignment:.leading,spacing:20) {
                HStack {VStack(alignment:.leading,spacing:5) {Text("MISSION CONTROL").font(.caption).tracking(2).foregroundStyle(.secondary);Text("Rover \(rover.id)").font(.largeTitle.weight(.semibold))};Spacer();Label(rover.membership.capitalized,systemImage:"dot.radiowaves.left.and.right").font(.subheadline)}
                Group {
                    if geographic {GeographicMap(rovers:model.snapshot.rovers,selected:rover.id,latitude:anchorLatitude,longitude:anchorLongitude,draft:coordinateDraft) {lat,lon in first=String(format:"%.7f",lat);second=String(format:"%.7f",lon)}}
                    else {LocalMap(rovers:model.snapshot.rovers,selected:rover.id,draft:localDraft) {x,y in first=String(format:"%.2f",x);second=String(format:"%.2f",y)}}
                }.frame(height:320).clipShape(RoundedRectangle(cornerRadius:12))
                HStack(alignment:.top,spacing:24) {
                    VStack(alignment:.leading,spacing:6) {
                        Text("POSITION").font(.caption).foregroundStyle(.secondary)
                        if let pose=rover.pose {Text(String(format:"x %.2f · y %.2f m",pose.x,pose.y)).monospacedDigit();Text(freshness(rover.poseAge)).font(.caption).foregroundStyle(.secondary)}else {Text("Waiting for pose").foregroundStyle(.secondary)}
                    }
                    Spacer()
                    VStack(alignment:.leading,spacing:6) {Text("GOAL").font(.caption).foregroundStyle(.secondary);Text(rover.goal?.state.capitalized ?? "Unknown");if let goal=rover.goal,goal.state != "idle" {Text(String(format:"%.2f m remaining",goal.distance)).monospacedDigit()};Text(freshness(rover.goalAge)).font(.caption).foregroundStyle(.secondary)}
                }
                Divider()
                VStack(alignment:.leading,spacing:12) {
                    Text("Set a waypoint").font(.title3.weight(.semibold))
                    Text("Tap the map to choose a target, or enter coordinates below.").font(.subheadline).foregroundStyle(.secondary)
                    HStack {
                        VStack(alignment:.leading) {Text(geographic ? "Latitude":"x · metres").font(.caption);TextField(geographic ? "38.82981":"12.0",text:$first).accessibilityIdentifier("target-first")}
                        VStack(alignment:.leading) {Text(geographic ? "Longitude":"y · metres").font(.caption);TextField(geographic ? "-77.3075":"0.0",text:$second).accessibilityIdentifier("target-second")}
                    }.textFieldStyle(.roundedBorder)
                    HStack {
                        Button("Send waypoint") {do {let waypoint=try WaypointDraft.make(first:first,second:second,geographic:geographic);Task {await model.send(id:rover.id,waypoint:waypoint)}}catch {model.error=error.localizedDescription}}
                            .buttonStyle(.borderedProminent).disabled(!ready || first.isEmpty || second.isEmpty || ["pending","cancelling"].contains(rover.commandPhase)).accessibilityIdentifier("send-waypoint")
                        Button("Cancel goal",role:.destructive) {Task {await model.cancel(id:rover.id)}}.buttonStyle(.bordered).disabled(!ready || rover.commandPhase=="cancelling").accessibilityIdentifier("cancel-goal")
                    }
                    if rover.commandPhase != "none" {Text(commandLabel).font(.subheadline).accessibilityIdentifier("command-phase")}
                    if let token=rover.commandToken ?? rover.goal?.token {Text("Token · \(token)").font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)}
                    if let notice=model.snapshot.notice {Text(notice).font(.caption).foregroundStyle(.secondary)}
                }
            }.padding(24)
        }.navigationTitle("Rover \(rover.id)")
        .onChange(of:rover.id) {_,_ in first="";second=""}
        .onChange(of:geographic) {_,_ in first="";second=""}
    }
    private var localDraft:(Double,Double)? {guard let a=Double(first),let b=Double(second),a.isFinite,b.isFinite else{return nil};return(a,b)}
    private var coordinateDraft:(Double,Double)? {guard let point=localDraft,abs(point.0)<=85,abs(point.1)<=180 else{return nil};return point}
    private func freshness(_ age:Double?)->String {guard let age else{return "No telemetry"};return String(format:age>=2.5 ? "Stale · %.1f s ago":"Updated %.1f s ago",age)}
    private var commandLabel:String {
        switch rover.commandPhase {case "pending":return "Sent · waiting for Terra acceptance";case "active":return "Waypoint accepted · driving";case "arrived":return "Arrived at waypoint";case "cancelling":return "Cancel sent · waiting for idle";case "cancelled":return "Goal cancelled · teleop released";case "unconfirmed":return "No acknowledgement · check Terra before sending again";case "superseded":return "Goal changed or cleared by Terra";case "failed":return "Publish failed";default:return rover.commandPhase.capitalized}
    }
}
