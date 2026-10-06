import SwiftUI
struct RoverDetailView:View {
    @EnvironmentObject var model:FleetModel
    let rover:RoverView
    @AppStorage("geographic") private var geographic=false
    @AppStorage("anchorLatitude") private var anchorLatitude=38.8297
    @AppStorage("anchorLongitude") private var anchorLongitude = -77.3075
    @FocusState private var drivingFocus:Bool
    @State private var first=""
    @State private var second=""
    private var ready:Bool {rover.membership=="online" && model.snapshot.link=="healthy" && !model.busy}
    var body:some View {
        ScrollView {
            VStack(alignment:.leading,spacing:20) {
                HStack {VStack(alignment:.leading,spacing:5) {Text("MISSION CONTROL").font(.caption).tracking(2).foregroundStyle(.secondary);Text("Rover \(rover.id)").font(.largeTitle.weight(.semibold))};Spacer();Label(rover.membership.capitalized,systemImage:"dot.radiowaves.left.and.right").font(.subheadline)}
                autonomyControls
                Group {
                    if geographic {GeographicMap(rovers:model.snapshot.rovers,selected:rover.id,latitude:anchorLatitude,longitude:anchorLongitude,draft:coordinateDraft) {lat,lon in first=String(format:"%.7f",lat);second=String(format:"%.7f",lon)}}
                    else {LocalMap(rovers:model.snapshot.rovers,selected:rover.id,occupancy:model.occupancy(rover.id),draft:localDraft) {x,y in first=String(format:"%.2f",x);second=String(format:"%.2f",y)}}
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
        .onDisappear {model.clearInput()}
        .onChange(of:drivingFocus) {_,focused in if !focused {model.clearInput()}}
        .onChange(of:rover.id) {_,_ in first="";second=""}
        .onChange(of:geographic) {_,_ in first="";second=""}
    }
    @ViewBuilder private var autonomyControls:some View {
        let state=model.state(rover.id)
        VStack(alignment:.leading,spacing:12) {
            if let mission=state?.mission {
                Text(mission.objective).font(.headline)
                Text("\(mission.confirmed)/\(mission.required) confirmed · \(Int(mission.remaining_seconds)) s remaining · \(mission.phase)")
                ForEach(mission.observations,id:\.survivor_id) {sighting in
                    HStack {Text(String(format:"Observed survivor %llu · %.1f, %.1f m",sighting.survivor_id,sighting.x,sighting.y));if !sighting.confirmed {Button("Confirm report") {Task {await model.action(id:sighting.rover_id,kind:"mission/report",values:["survivor_id":sighting.survivor_id])}}}else{Text("Confirmed").foregroundStyle(.secondary)}}
                }
            }
            if state?.recording=="incomplete" {Text("Session recording incomplete · controls remain available").foregroundStyle(.orange)}
            if let status=state?.status {
                Text("Autonomy level").font(.headline)
                Picker("Autonomy",selection:Binding(get:{status.requested_level},set:{level in model.clearInput();Task {await model.action(id:rover.id,kind:"autonomy",values:["level":level])}})) {
                    ForEach(status.supported_levels,id:\.self) {level in Text(level.replacingOccurrences(of:"_",with:" ").capitalized).tag(level)}
                }.pickerStyle(.menu).accessibilityIdentifier("autonomy-level").disabled(!ready || (state?.age ?? .infinity)>=2.5 || state?.action_phase=="pending" || model.changingAuthority.contains(rover.id))
                Text("Effective: \(status.effective_level ?? "Held") · \(status.reason.replacingOccurrences(of:"_",with:" "))").font(.subheadline).accessibilityElement(children:.ignore).accessibilityLabel("Effective: \(status.effective_level ?? "Held") · \(status.reason.replacingOccurrences(of:"_",with:" "))").accessibilityIdentifier("effective-authority")
                if let assigned=status.assigned_level {Text("Assigned condition: \(assigned)").font(.caption).foregroundStyle(.secondary)}
                if let reason=status.request_reason {Text(reason.replacingOccurrences(of:"_",with:" ")).font(.caption).foregroundStyle(.secondary)}
                Text("Change: \(state?.action_phase ?? "none")").font(.caption).accessibilityElement(children:.ignore).accessibilityLabel("Change: \(state?.action_phase ?? "none")").accessibilityIdentifier("autonomy-request-status")
            }else {Text("Waiting for autonomy capability/status").foregroundStyle(.secondary)}
            HStack {
                Button("Take over") {model.clearInput();Task {await model.action(id:rover.id,kind:"autonomy",values:["level":"teleop"])}}.accessibilityIdentifier("take-over")
                Button("Emergency stop",role:.destructive) {model.clearInput();Task {await model.action(id:rover.id,kind:"safety",values:["action":"stop"])}}.accessibilityIdentifier("emergency-stop")
                if state?.status?.safety=="emergency_stop" {Button("Reset stop") {Task {await model.action(id:rover.id,kind:"safety",values:["action":"reset"])}}}
            }.buttonStyle(.bordered).disabled(model.snapshot.link=="disconnected")
            if let proposal=state?.proposal,state?.status?.requested_level=="supervised" {
                Text(String(format:"Search proposal · %.1f, %.1f m",proposal.x,proposal.y))
                HStack {Button("Approve") {Task {await model.action(id:rover.id,kind:"goal/decision",values:["decision":"approve","proposal_id":proposal.proposal_id,"run_id":proposal.run_id])}};Button("Reject") {Task {await model.action(id:rover.id,kind:"goal/decision",values:["decision":"reject","proposal_id":proposal.proposal_id,"run_id":proposal.run_id])}}}.disabled(!ready || (state?.age ?? .infinity)>=2.5 || (state?.proposal_remaining ?? 0)<=0)
            }
            if state?.status?.paused==true {Button("Resume search proposals") {Task {await model.action(id:rover.id,kind:"goal/decision",values:["decision":"resume","run_id":state?.status?.run_id ?? ""])}}}
            if state?.canDrive==true {
                VStack(alignment:.leading,spacing:8) {
                    Text("Hold to drive · tap here for W/A/S/D or arrow keys").font(.subheadline)
                    HStack {drivePad("Forward","w");drivePad("Reverse","s");drivePad("Left","a");drivePad("Right","d")}
                }.padding(12).background(.quaternary,in:RoundedRectangle(cornerRadius:8))
                .focusable().focused($drivingFocus)
                .onKeyPress(phases:[.down,.up]) {press in
                    let key:String
                    switch press.key {case .upArrow:key="w";case .downArrow:key="s";case .leftArrow:key="a";case .rightArrow:key="d";default:key=press.characters.lowercased()}
                    guard ["w","a","s","d"].contains(key) else{return .ignored}
                    model.input(id:rover.id,key:key,down:press.phase == .down);return .handled
                }
                .onDisappear {model.clearInput()}
                #if os(macOS)
                .onReceive(NotificationCenter.default.publisher(for:NSApplication.didResignActiveNotification)) {_ in drivingFocus=false;model.clearInput()}
                #endif
            }
        }
    }
    private func drivePad(_ label:String,_ key:String)->some View {
        Text(label).padding(10).background(.quaternary,in:RoundedRectangle(cornerRadius:6))
            .accessibilityLabel("Hold to drive \(label.lowercased())")
            .gesture(DragGesture(minimumDistance:0).onChanged {_ in model.input(id:rover.id,key:key,down:true)}.onEnded {_ in model.input(id:rover.id,key:key,down:false)})
    }
    private var localDraft:(Double,Double)? {guard let a=Double(first),let b=Double(second),a.isFinite,b.isFinite else{return nil};return(a,b)}
    private var coordinateDraft:(Double,Double)? {guard let point=localDraft,abs(point.0)<=85,abs(point.1)<=180 else{return nil};return point}
    private func freshness(_ age:Double?)->String {guard let age else{return "No telemetry"};return String(format:age>=2.5 ? "Stale · %.1f s ago":"Updated %.1f s ago",age)}
    private var commandLabel:String {
        switch rover.commandPhase {case "pending":return "Sent · waiting for Terra acceptance";case "active":return "Waypoint accepted · driving";case "arrived":return "Arrived at waypoint";case "cancelling":return "Cancel sent · waiting for idle";case "cancelled":return "Goal cancelled · teleop released";case "unconfirmed":return "No acknowledgement · check Terra before sending again";case "superseded":return "Goal changed or cleared by Terra";case "failed":return "Publish failed";default:return rover.commandPhase.capitalized}
    }
}
