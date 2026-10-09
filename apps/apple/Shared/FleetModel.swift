import Foundation
import SwiftUI
import MapKit
protocol FleetBackend: Sendable {
    func connect(endpoint: String, prefix: String) async throws
    func disconnect() async
    func operatorTelemetry() async -> String
    func operatorCommand(id: UInt64, kind: String, payload: String) async throws
    func localization() async -> String
    func snapshot() async -> FleetSnapshot
    func send(id: UInt64, waypoint: Waypoint) async throws -> String
    func occupancyState() async -> String
    func operatorState() async -> String
    func action(id:UInt64,kind:String,payload:String) async throws
    func fleetAction(kind:String,payload:String) async throws -> String
    func input(id:UInt64,key:String,down:Bool,sequence:UInt64,runId:String,revision:UInt64) async throws
    func clearInput(sequence:UInt64) async
    func exportLog(path:String) async throws
    func cancel(id: UInt64) async throws
}
extension FleetBackend {
    func occupancyState() async -> String {"{}"}
    func operatorState() async -> String {"{}"}
    func action(id:UInt64,kind:String,payload:String) async throws {}
    func fleetAction(kind:String,payload:String) async throws -> String {"[]"}
    func input(id:UInt64,key:String,down:Bool,sequence:UInt64,runId:String,revision:UInt64) async throws {}
    func clearInput(sequence:UInt64) async {}
    func exportLog(path:String) async throws {}

    func localization() async -> String { "{}" }
    func operatorTelemetry() async -> String { "{}" }
    func operatorCommand(id: UInt64, kind: String, payload: String) async throws { throw DraftError.invalid }
}
actor NativeBackend: FleetBackend {
    private let client = ArgosClient()
    func occupancyState() async -> String {client.occupancySnapshot()}
    func operatorState() async -> String {client.operatorSnapshot()}
    func action(id:UInt64,kind:String,payload:String) async throws {_ = try client.operatorAction(roverId:id,kind:kind,payload:payload)}
    func fleetAction(kind:String,payload:String) async throws -> String {try client.fleetAction(kind:kind,payload:payload)}
    func input(id:UInt64,key:String,down:Bool,sequence:UInt64,runId:String,revision:UInt64) async throws {try client.inputEvent(roverId:id,key:key,down:down,sequence:sequence,runId:runId,revision:revision)}
    func clearInput(sequence:UInt64) async {client.clearInputEvent(sequence:sequence)}
    func exportLog(path:String) async throws {let client=self.client;try await Task.detached {try client.exportSession(destination:path)}.value}


    func connect(endpoint: String, prefix: String) async throws {try client.connect(config: ConnectionConfig(endpoint:endpoint,prefix:prefix))}
    func disconnect() async {client.disconnect()}
    func operatorTelemetry() async -> String {client.operatorSnapshot()}
    func operatorCommand(id: UInt64, kind: String, payload: String) async throws {try client.operatorCommand(roverId: id, kind: kind, payload: payload)}
    func localization() async -> String {client.localizationSnapshot()}
    func snapshot() async -> FleetSnapshot {client.snapshot()}
    func send(id: UInt64, waypoint: Waypoint) async throws -> String {try client.sendGoal(roverId:id,waypoint:waypoint)}
    func cancel(id: UInt64) async throws {try client.cancelGoal(roverId:id)}
}
@MainActor final class FleetModel: ObservableObject {
    @Published private(set) var snapshot = FleetSnapshot(link:"disconnected",fleetAge:nil,rovers:[],notice:nil)
    @Published private(set) var localizations: [UInt64: GeographicReference] = [:]
    @Published private(set) var geographicOrigin: CLLocationCoordinate2D?
    @Published private(set) var frameKey = "local"
    @Published private(set) var sceneRovers: [RoverView] = []
    @Published private(set) var hardware: [UInt64: HardwareState] = [:]
    @Published private(set) var hardwarePending: [UInt64: Date] = [:]
    @Published private(set) var searchReports: [UInt64: [TargetSearchReport]] = [:]
    private var reportAckTimes: [String:Date] = [:]
    @Published private(set) var searches: [UInt64: TargetSearchState] = [:]
    @Published private(set) var searchPending: [UInt64: (token:String,started:Date)] = [:]
    @Published private(set) var authorities: [UInt64: VehicleAuthority] = [:]
    @Published private(set) var occupancyMaps: [UInt64: OccupancyFrame] = [:]
    @Published private(set) var clouds: [UInt64: PointCloudFrame] = [:]
    @Published private(set) var autonomyPending: [UInt64: String] = [:]
    @Published private(set) var busy = false
    @Published var error: String?
    @Published var selected: UInt64? {didSet {if oldValue != selected {clearInput()}}}
    @Published private(set) var operatorStates:[String:OperatorState]=[:]
    @Published private(set) var occupancyStates:[String:OccupancyState]=[:]
    @Published private(set) var changingAuthority:Set<UInt64>=[]
    @Published var fleetResult:String?
    @Published var exportedLog:URL?
    private var inputSequence:UInt64=0
    private let backend: any FleetBackend
    private var observation: Task<Void,Never>?
    var observing: Bool {observation != nil}
    init(backend: any FleetBackend = NativeBackend()) {self.backend=backend}
    func connect(endpoint:String,prefix:String) async {
        guard !busy else {return}
        busy=true;error=nil;stopObservation()
        defer {busy=false}
        do {try await backend.connect(endpoint:endpoint,prefix:prefix);await refresh();startObservation()}
        catch {self.error=error.localizedDescription;await refresh()}
    }
    func disconnect() async {stopObservation();await backend.disconnect();await refresh()}
    func refresh() async {
        snapshot=await backend.snapshot()
        let data=Data((await backend.operatorState()).utf8)
        occupancyStates=(try? JSONDecoder().decode([String:OccupancyState].self,from:Data((await backend.occupancyState()).utf8))) ?? [:]
        let previous=operatorStates
        operatorStates=(try? JSONDecoder().decode([String:OperatorState].self,from:data)) ?? [:]
        if let selected,(previous[String(selected)]?.status?.revision != operatorStates[String(selected)]?.status?.revision || previous[String(selected)]?.status?.run_id != operatorStates[String(selected)]?.status?.run_id) {clearInput()}

        let operatorPayload = await backend.operatorTelemetry()
        let operatorData = (try? JSONDecoder().decode([String: OperatorTelemetry].self, from: Data(operatorPayload.utf8))) ?? [:]
        hardware = Dictionary(uniqueKeysWithValues: operatorData.compactMap { key, value in
            guard let id = UInt64(key), let state = value.hardware, state.fresh else { return nil }; return (id, state)
        })
        for (id, started) in hardwarePending {
            if hardware[id]?.armed == true || hardware[id]?.arming == true || hardware[id]?.result?.hasPrefix("rejected") == true {
                hardwarePending.removeValue(forKey: id)
            } else if Date().timeIntervalSince(started) >= 4 {
                hardwarePending.removeValue(forKey: id)
                error = "Arm not confirmed · \(hardware[id]?.reason ?? "Hardware status unavailable")"
            }
        }
        authorities = Dictionary(uniqueKeysWithValues: operatorData.compactMap { key, value in
            guard let id = UInt64(key), let authority = value.authority else { return nil }; return (id, authority)
        })
        searches = Dictionary(uniqueKeysWithValues: operatorData.compactMap { key,value in
            guard let id=UInt64(key),let search=value.search,search.runID==authorities[id]?.runID else {return nil};return (id,search)
        })
        for (key,data) in operatorData {
            guard let id=UInt64(key) else {continue}
            for report in data.reports ?? [] {
                let identity="\(id):\(report.runID):\(report.searchID):\(report.reportID)"
                if !(searchReports[id] ?? []).contains(where:{$0.runID==report.runID && $0.searchID==report.searchID && $0.reportID==report.reportID}) {
                    var history=searchReports[id] ?? [];history.append(report);searchReports[id]=Array(history.suffix(128))
                }
                if reportAckTimes[identity].map({Date().timeIntervalSince($0)<1}) == true {continue}
                let ack:[String:Any]=["version":1,"run_id":report.runID,"search_id":report.searchID,"report_id":report.reportID,"token":UUID().uuidString]
                if let payload=try? JSONSerialization.data(withJSONObject:ack) {do{try await backend.operatorCommand(id:id,kind:"search/report/ack",payload:String(decoding:payload,as:UTF8.self));reportAckTimes[identity]=Date()}catch{self.error="Report acknowledgement failed · \(error.localizedDescription)"}}
            }
        }
        reportAckTimes=reportAckTimes.filter{Date().timeIntervalSince($0.value)<5}
        for (id,pending) in searchPending {
            if authorities[id]?.token==pending.token {searchPending.removeValue(forKey:id);if authorities[id]?.result=="rejected" {error="Search request rejected · \(authorities[id]?.reason ?? "unknown")"}}
            else if Date().timeIntervalSince(pending.started)>4 {searchPending.removeValue(forKey:id);error="Search request unconfirmed · check rover status"}
        }
        clouds = Dictionary(uniqueKeysWithValues: operatorData.compactMap { key, value in
            guard let id = UInt64(key), let cloud = value.cloud, cloud.usable else { return nil }; return (id, cloud)
        })
        occupancyMaps = Dictionary(uniqueKeysWithValues: operatorData.compactMap { key, value in
            guard let id = UInt64(key), let map = value.occupancy, map.usable, map.roverID == id, map.runID == authorities[id]?.runID else { return nil }; return (id, map)
        })
        for (id, token) in autonomyPending where authorities[id]?.token == token { autonomyPending.removeValue(forKey: id) }
        let payload = await backend.localization()
        let decoded = (try? JSONDecoder().decode([String: GeographicReference].self, from: Data(payload.utf8))) ?? [:]
        localizations = Dictionary(uniqueKeysWithValues: decoded.compactMap { key, value in UInt64(key).map { ($0, value) } })
        let located = snapshot.rovers.filter { $0.pose != nil }
        let geographic = !located.isEmpty && located.allSatisfy { $0.membership == "online" && ($0.poseAge ?? .infinity) < 2.5 && localizations[$0.id]?.usable == true }
        let nextKey = geographic ? located.sorted { $0.id < $1.id }.map { "\($0.id):\(localizations[$0.id]!.frameID)" }.joined(separator: ",") : "local:" + snapshot.rovers.sorted { $0.id < $1.id }.map { "\($0.id):\(localizations[$0.id]?.frameID ?? "unknown")" }.joined(separator: ",")
        if frameKey != nextKey {
            frameKey = nextKey
            geographicOrigin = geographic ? localizations[located.sorted { $0.id < $1.id }[0].id]?.origin : nil
        }
        sceneRovers = snapshot.rovers.map { rover in
            guard let origin = geographicOrigin, let ref = localizations[rover.id], ref.usable else { return rover }
            var result = rover
            if let pose = rover.pose {
                let point = ref.scene(x: pose.x, y: pose.y, origin: origin)
                result.pose = RoverPose(x: point.x, y: point.y, yaw: pose.yaw + ref.rotation!)
            }
            if var goal = rover.goal {
                let point = ref.scene(x: goal.x, y: goal.y, origin: origin)
                goal.x = point.x; goal.y = point.y; result.goal = goal
            }
            return result
        }
        if let selected, !snapshot.rovers.contains(where:{$0.id==selected}) {self.selected=nil}
        if selected == nil {selected=snapshot.rovers.first(where:{$0.membership=="online"})?.id ?? snapshot.rovers.first?.id}
    }
    func motorsArmed(_ id: UInt64) -> Bool { hardware[id]?.fresh == true && hardware[id]?.armed == true }
    func requestHardware(id: UInt64, arm: Bool) async {
        guard !arm || (driveAvailable(id) && hardware[id]?.ready == true) else { error = "Fresh, ready rover required to arm"; return }
        do {
            var payload: [String: Any] = ["action": arm ? "arm" : "disarm", "token": UUID().uuidString]
            if arm {
                guard let authority = authorities[id] else { error = "Vehicle authority unavailable"; return }
                payload["run_id"] = authority.runID; payload["authority_revision"] = authority.revision
            }
            try await backend.operatorCommand(id: id, kind: "hardware", payload: String(decoding: JSONSerialization.data(withJSONObject: payload), as: UTF8.self))
            if arm { hardwarePending[id] = Date() } else { hardwarePending.removeValue(forKey: id) }
            error = nil
        } catch { self.error = error.localizedDescription }
    }
    func driveAvailable(_ id: UInt64) -> Bool {
        snapshot.link == "healthy" && authorities[id]?.fresh == true && snapshot.rovers.contains { $0.id == id && $0.membership == "online" && ($0.poseAge ?? .infinity) < 0.5 }
    }
    func selectAutonomy(id: UInt64, level: String) async -> String? {
        guard driveAvailable(id), authorities[id]?.supportedLevels.contains(level) == true else { error = "Fresh vehicle authority and supported level required"; return nil }
        let token = UUID().uuidString
        do {
            let data = try JSONSerialization.data(withJSONObject: ["level":level,"token":token])
            try await backend.operatorCommand(id: id, kind: "autonomy", payload: String(decoding: data, as: UTF8.self))
            autonomyPending[id] = token
            return token
        } catch { self.error = error.localizedDescription; return nil }
    }
    func startSearch(id: UInt64, targetClass: String, minX: Double, minY: Double, maxX: Double, maxY: Double, budget: Double) async {
        guard driveAvailable(id), let authority=authorities[id],authority.requestedLevel=="target_search",authority.supportedLevels.contains("target_search") else {error="Select L4 and wait for rover acknowledgement";return}
        guard [minX,minY,maxX,maxY,budget].allSatisfy({$0.isFinite}),maxX>minX,maxY>minY,maxX-minX<=100,maxY-minY<=100,budget>0,budget<=3600,!targetClass.isEmpty else {error="Enter a target, valid local bounds and a budget up to 3600 seconds";return}
        let token=UUID().uuidString
        let payload:[String:Any] = ["version":1,"run_id":authority.runID,"search_id":UUID().uuidString,"token":token,"target_class":targetClass,"bounds":["min_x":minX,"min_y":minY,"max_x":maxX,"max_y":maxY],"time_budget_s":budget]
        await publishSearch(id:id,kind:"search",payload:payload,token:token)
    }
    func searchAction(id: UInt64, action: String) async {
        guard let search=searches[id],search.fresh else {error="Fresh search status required";return}
        let token=UUID().uuidString
        await publishSearch(id:id,kind:"search/action",payload:["version":1,"run_id":search.runID,"search_id":search.searchID,"token":token,"action":action],token:token)
    }
    private func publishSearch(id: UInt64, kind: String, payload: [String:Any], token: String) async {
        do {try await backend.operatorCommand(id:id,kind:kind,payload:String(decoding:JSONSerialization.data(withJSONObject:payload),as:UTF8.self));searchPending[id]=(token,Date());error=nil}
        catch {self.error=error.localizedDescription}
    }
    func drive(id: UInt64, vector: DriveVector, authority: VehicleAuthority, session: String, sequence: UInt64) async throws {
        guard vector == .zero || motorsArmed(id) else { throw NSError(domain: "ARGOS", code: 1, userInfo: [NSLocalizedDescriptionKey: "Motors are not confirmed armed"]) }
        let value: [String: Any] = ["linear":vector.linear,"angular":vector.angular,"run_id":authority.runID,
            "authority_revision":authority.revision,"operator_session_id":session,"sequence":sequence]
        let data = try JSONSerialization.data(withJSONObject: value)
        try await backend.operatorCommand(id: id, kind: "teleop", payload: String(decoding: data, as: UTF8.self))
    }
    func mapScenePoint(id: UInt64, x: Double, y: Double) -> (x: Double, y: Double)? {
        if let origin = geographicOrigin {
            guard let ref = localizations[id], ref.usable else { return nil }
            return ref.scene(x: x, y: y, origin: origin)
        }
        return (x,y)
    }
    func waypointCell(id: UInt64, x: Double, y: Double) -> Int? {
        guard let map = occupancyMaps[id], map.usable else { return nil }
        var local = (x:x,y:y)
        if let origin = geographicOrigin {
            guard let ref = localizations[id], ref.usable else { return nil }
            local = ref.local(x:x,y:y,origin:origin)
        }
        return map.value(x:local.x,y:local.y)
    }
    func sceneCloud(_ id: UInt64) -> PointCloudFrame? {
        guard var cloud = clouds[id] else { return nil }
        if let ref = localizations[id], cloud.frameID != ref.frameID { return nil }
        if let origin = geographicOrigin {
            guard let ref = localizations[id], ref.usable, cloud.frameID == ref.frameID else { return nil }
            cloud.points = cloud.points.map { point in
                let transformed = ref.scene(x: point[0], y: point[1], origin: origin)
                return [transformed.x, transformed.y, point[2]]
            }
        }
        cloud.frameID += ":\(id):" + frameKey
        return cloud
    }
    func canPlaceGeographically(_ id: UInt64) -> Bool { geographicOrigin == nil || localizations[id]?.usable == true }
    func state(_ id:UInt64)->OperatorState? {operatorStates[String(id)]}
    func occupancy(_ id:UInt64)->OccupancyState? {
        guard let map=occupancyStates[String(id)],map.grid.rover_id==id,map.grid.valid,let run=state(id)?.status?.run_id,run==map.grid.run_id else{return nil};return map
    }
    func clearInput(){inputSequence+=1;let sequence=inputSequence;Task {await backend.clearInput(sequence:sequence)}}
    func input(id:UInt64,key:String,down:Bool){guard let authority=state(id)?.status,let run=authority.run_id else{return};inputSequence+=1;let sequence=inputSequence;Task {do {try await backend.input(id:id,key:key,down:down,sequence:sequence,runId:run,revision:authority.revision)}catch {self.error=error.localizedDescription}}}
func action(id:UInt64,kind:String,values:[String:Any]) async {
    if kind=="autonomy" {guard !changingAuthority.contains(id) else{return};changingAuthority.insert(id)}
    defer {if kind=="autonomy" {changingAuthority.remove(id)}}
    do {let data=try JSONSerialization.data(withJSONObject:values);try await backend.action(id:id,kind:kind,payload:String(decoding:data,as:UTF8.self));await refresh()}catch{self.error=error.localizedDescription}
}
    func fleetAction(stop:Bool) async {clearInput();do {fleetResult=try await backend.fleetAction(kind:stop ? "safety":"autonomy",payload:stop ? "{\"action\":\"stop\"}":"{\"level\":\"teleop\"}");await refresh()}catch{self.error=error.localizedDescription}}
    func exportLog() async {do {let url=FileManager.default.temporaryDirectory.appendingPathComponent("ARGOS-session-\(UUID().uuidString).jsonl");try await backend.exportLog(path:url.path);exportedLog=url}catch{self.error=error.localizedDescription}}
    func startObservation() {
        guard observation == nil, snapshot.link != "disconnected" else {return}
        observation=Task { [weak self] in
            while !Task.isCancelled {
                guard let self else {return}
                await self.refresh()
                do {try await Task.sleep(for:.milliseconds(200))} catch {return}
            }
        }
    }
    func stopObservation() {observation?.cancel();observation=nil}
    func send(id:UInt64,waypoint:Waypoint) async {
        guard motorsArmed(id) else { error = "Arm the rover before executing a waypoint"; return }
        guard !busy else {return};busy=true;error=nil;defer{busy=false}
        do {
            if authorities[id]?.requestedLevel != "waypoint" {
                guard let token = await selectAutonomy(id: id, level: "waypoint") else { return }
                let deadline = Date().addingTimeInterval(2)
                var accepted = false
                while Date() < deadline {
                    await refresh()
                    if let authority = authorities[id], authority.token == token {
                        guard authority.result == "accepted", authority.requestedLevel == "waypoint" else {
                            error = "Waypoint mode rejected · \(authority.reason)"; return
                        }
                        accepted = true; break
                    }
                    try await Task.sleep(for: .milliseconds(100))
                }
                guard accepted else { error = "Waypoint mode was not acknowledged"; return }
            }
            guard motorsArmed(id), driveAvailable(id) else { error = "Rover lost readiness before waypoint execution"; return }
            _ = try await backend.send(id:id,waypoint:waypoint)
            await refresh()
        } catch { self.error=error.localizedDescription }
    }
    func cancel(id:UInt64) async {
        guard !busy else {return};busy=true;error=nil;defer{busy=false}
        do {try await backend.cancel(id:id);await refresh()} catch {self.error=error.localizedDescription}
    }
}
enum DraftError: LocalizedError {case invalid;var errorDescription:String? {"Enter two finite numeric coordinates."}}
enum WaypointDraft {
    static func make(first:String,second:String,geographic:Bool) throws -> Waypoint {
        guard let a=Double(first.trimmingCharacters(in:.whitespaces)),let b=Double(second.trimmingCharacters(in:.whitespaces)),a.isFinite,b.isFinite else {throw DraftError.invalid}
        return geographic ? .geographic(latitude:a,longitude:b,yaw:nil):.local(x:a,y:b,yaw:nil)
    }
}
// Same local tangent plane as terra-waypoint; +x north, +y west.
enum MapProjection {
    static let scale=40_075_016.686/360.0
    static func coordinate(x:Double,y:Double,latitude:Double,longitude:Double)->CLLocationCoordinate2D {
        CLLocationCoordinate2D(latitude:latitude+x/scale,longitude:longitude-y/(scale*cos(latitude * .pi/180)))
    }
    static func local(_ coordinate:CLLocationCoordinate2D,latitude:Double,longitude:Double)->(x:Double,y:Double) {
        ((coordinate.latitude-latitude)*scale,-(coordinate.longitude-longitude)*scale*cos(latitude * .pi/180))
    }
}

// Attention is derived from observed transport state, never a fabricated health score.
enum DashboardPresentation {
    static func attentionReason(_ rover: RoverView) -> String? {
        if rover.commandPhase == "failed" { return "Command failed" }
        if rover.commandPhase == "unconfirmed" { return "Command unconfirmed" }
        if rover.membership != "online" { return "Vehicle \(rover.membership)" }
        guard let poseAge = rover.poseAge else { return "Position unavailable" }
        if poseAge >= 2.5 { return "Position is stale" }
        if rover.goal != nil {
            guard let goalAge = rover.goalAge else { return "Goal status unavailable" }
            if goalAge >= 2.5 { return "Goal status is stale" }
        }
        return nil
    }
    static func attentionPriority(_ rover: RoverView) -> Int {
        if rover.commandPhase == "failed" { return 0 }
        if rover.commandPhase == "unconfirmed" { return 1 }
        if rover.membership != "online" { return 2 }
        if rover.poseAge == nil { return 3 }
        if (rover.poseAge ?? 0) >= 2.5 { return 4 }
        return 5
    }
}

// Display alignment is separate from the rover's unchanged control frame.
struct GeographicReference: Codable {
    var version: Int
    var frameID: String
    var mode: String
    var reason: String
    var tracking: String
    var age: Double
    var gpsAccuracy: Double?
    var headingAccuracy: Double?
    var originLatitude: Double?
    var originLongitude: Double?
    var anchorX: Double?
    var anchorY: Double?
    var rotation: Double?
    var usable: Bool {
        version == 1 && !frameID.isEmpty && mode == "geographic" && tracking == "normal" && age.isFinite && age >= 0 && age < 2.5
        && gpsAccuracy.map { $0.isFinite && $0 >= 0 && $0 <= 10 } == true
        && headingAccuracy.map { $0.isFinite && $0 >= 0 && $0 <= 15 } == true
        && originLatitude.map { $0.isFinite && abs($0) <= 85 } == true
        && originLongitude.map { $0.isFinite && abs($0) <= 180 } == true
        && anchorX?.isFinite == true && anchorY?.isFinite == true && rotation?.isFinite == true
    }
    var origin: CLLocationCoordinate2D? {
        guard usable else { return nil }
        return CLLocationCoordinate2D(latitude: originLatitude!, longitude: originLongitude!)
    }
    func scene(x: Double, y: Double, origin: CLLocationCoordinate2D) -> (x: Double, y: Double) {
        let dx = x - anchorX!, dy = y - anchorY!, r = rotation!
        let coordinate = MapProjection.coordinate(x: cos(r)*dx-sin(r)*dy, y: sin(r)*dx+cos(r)*dy, latitude: originLatitude!, longitude: originLongitude!)
        return MapProjection.local(coordinate, latitude: origin.latitude, longitude: origin.longitude)
    }
    func local(x: Double, y: Double, origin: CLLocationCoordinate2D) -> (x: Double, y: Double) {
        let coordinate = MapProjection.coordinate(x: x, y: y, latitude: origin.latitude, longitude: origin.longitude)
        let p = MapProjection.local(coordinate, latitude: originLatitude!, longitude: originLongitude!)
        let r = rotation!
        return (anchorX! + cos(r)*p.x+sin(r)*p.y, anchorY! - sin(r)*p.x+cos(r)*p.y)
    }
}

struct OperatorState:Decodable {
    var recording:String?;var proposal_remaining:Double?;var status:AuthorityState?;var age:Double?;var proposal:ProposalState?;var mission:MissionState?;var action_phase:String?;var action_token:String?
    var canDrive:Bool {(age ?? .infinity)<2.5 && status?.safety=="clear" && ["teleop","assisted_teleop"].contains(status?.effective_level ?? "") && ["none","accepted","rejected"].contains(action_phase ?? "none")}
}
struct AuthorityState:Decodable {var run_id:String?;var assigned_level:String?;var requested_level:String;var effective_level:String?;var active_source:String;var safety:String;var reason:String;var revision:UInt64;var supported_levels:[String];var paused:Bool;var request_reason:String?}
struct ProposalState:Decodable {var run_id:String;var proposal_id:UInt64;var x:Double;var y:Double;var expires_at:Double;var reason:String}
struct MissionState:Decodable {var mission_id:String;var objective:String;var phase:String;var confirmed:Int;var required:Int;var remaining_seconds:Double;var observations:[MissionObservation]}
struct MissionObservation:Decodable {var survivor_id:UInt64;var x:Double;var y:Double;var rover_id:UInt64;var confirmed:Bool}

struct OccupancyState:Decodable {
    let grid:OccupancyGridState
    let age:Double
    var stale:Bool {age>=2.5}
}
struct OccupancyGridState:Decodable {
    let schema_version:Int;let rover_id:UInt64;let run_id:String;let sequence:UInt64
    let width:Int;let height:Int;let resolution:Double;let origin_x:Double;let origin_y:Double;let occupancy:[Int]
    var valid:Bool {schema_version==1 && width>0 && height>0 && width<=250000 && height<=250000 && width*height<=250000 && occupancy.count==width*height && resolution.isFinite && resolution>0 && origin_x.isFinite && origin_y.isFinite && occupancy.allSatisfy {(-1...100).contains($0)}}
}
enum LocalMapProjection {
    static func screen(x:Double,y:Double,size:CGSize,extent:Double,centerX:Double=0,centerY:Double=0)->CGPoint {
        let scale=min(size.width,size.height)/(2*extent)
        return CGPoint(x:size.width/2-(y-centerY)*scale,y:size.height/2-(x-centerX)*scale)
    }
    static func world(_ point:CGPoint,size:CGSize,extent:Double,centerX:Double=0,centerY:Double=0)->(Double,Double) {
        let scale=min(size.width,size.height)/(2*extent)
        return (centerX+(size.height/2-point.y)/scale,centerY+(size.width/2-point.x)/scale)
    }
}
