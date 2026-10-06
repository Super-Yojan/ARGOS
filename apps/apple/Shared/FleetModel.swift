import Foundation
import SwiftUI
import MapKit
protocol FleetBackend: Sendable {
    func connect(endpoint: String, prefix: String) async throws
    func disconnect() async
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
    func snapshot() async -> FleetSnapshot {client.snapshot()}
    func send(id: UInt64, waypoint: Waypoint) async throws -> String {try client.sendGoal(roverId:id,waypoint:waypoint)}
    func cancel(id: UInt64) async throws {try client.cancelGoal(roverId:id)}
}
@MainActor final class FleetModel: ObservableObject {
    @Published private(set) var snapshot = FleetSnapshot(link:"disconnected",fleetAge:nil,rovers:[],notice:nil)
    @Published private(set) var busy = false
    @Published var error: String?
    @Published var selected: UInt64? {didSet {if oldValue != selected {clearInput()}}}
    @Published private(set) var operatorStates:[String:OperatorState]=[:]
    @Published private(set) var occupancyStates:[String:OccupancyState]=[:]
    @Published private(set) var changingAuthority:Set<UInt64>=[]
    @Published var fleetResult:String?
    @Published var exportedLog:URL?
    private let backend: any FleetBackend
    private var inputSequence:UInt64=0
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
        if let selected, !snapshot.rovers.contains(where:{$0.id==selected}) {self.selected=nil}
        if selected == nil {selected=snapshot.rovers.first(where:{$0.membership=="online"})?.id}
    }
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
    func send(id:UInt64,waypoint:Waypoint) async {
        guard !busy else {return};busy=true;error=nil;defer{busy=false}
        do {_ = try await backend.send(id:id,waypoint:waypoint);await refresh()} catch {self.error=error.localizedDescription}
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
