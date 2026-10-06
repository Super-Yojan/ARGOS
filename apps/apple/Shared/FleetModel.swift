import Foundation
import SwiftUI
import MapKit
protocol FleetBackend: Sendable {
    func connect(endpoint: String, prefix: String) async throws
    func disconnect() async
    func snapshot() async -> FleetSnapshot
    func send(id: UInt64, waypoint: Waypoint) async throws -> String
    func cancel(id: UInt64) async throws
}
actor NativeBackend: FleetBackend {
    private let client = ArgosClient()
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
    @Published var selected: UInt64?
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
        if let selected, !snapshot.rovers.contains(where:{$0.id==selected && $0.membership=="online"}) {self.selected=nil}
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
