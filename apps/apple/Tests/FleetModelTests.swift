import XCTest
#if canImport(ARGOS)
@testable import ARGOS
#endif
final class FleetModelTests: XCTestCase {
    func testDraftRejectsNonfiniteInputAndKeepsWestAxis() throws {
        XCTAssertThrowsError(try WaypointDraft.make(first:"nan", second:"0", geographic:false))
        XCTAssertThrowsError(try WaypointDraft.make(first:"", second:"0", geographic:false))
        let waypoint=try WaypointDraft.make(first:"12",second:"-4",geographic:false)
        XCTAssertEqual(waypoint,.local(x:12,y:-4,yaw:nil))
        let coordinate=MapProjection.coordinate(x:12,y:4,latitude:38.8297,longitude:-77.3075)
        XCTAssertGreaterThan(coordinate.latitude,38.8297)
        XCTAssertLessThan(coordinate.longitude,-77.3075)
        let local=MapProjection.local(coordinate,latitude:38.8297,longitude:-77.3075)
        XCTAssertEqual(local.x,12,accuracy:0.001)
        XCTAssertEqual(local.y,4,accuracy:0.001)
    }
    @MainActor func testDisconnectStopsObservationAndSendDoesNotRepeat() async throws {
        let backend=FakeBackend()
        let model=FleetModel(backend:backend)
        await model.connect(endpoint:"tcp/127.0.0.1:7447",prefix:"terra/rover")
        await model.refresh()
        XCTAssertEqual(model.snapshot.link,"healthy")
        await model.send(id:8,waypoint:.local(x:12,y:0,yaw:nil))
        await model.refresh();await model.refresh()
        let count=await backend.sendCount
        XCTAssertEqual(count,1)
        await model.disconnect()
        XCTAssertEqual(model.snapshot.link,"disconnected")
        XCTAssertFalse(model.observing)
    }
    @MainActor func testStaleSelectedRoverRemainsInspectable() async {
        let backend=FakeBackend()
        let model=FleetModel(backend:backend)
        await model.connect(endpoint:"tcp/127.0.0.1:7447",prefix:"terra/rover")
        await model.refresh()
        XCTAssertEqual(model.selected,8)
        await backend.setMembership("stale")
        await model.refresh()
        XCTAssertEqual(model.selected,8)
        await backend.setMembership("absent")
        await model.refresh()
        XCTAssertEqual(model.selected,8)
        await model.disconnect()
    }
}
actor FakeBackend
: FleetBackend {
    var sendCount=0
    var connected=false
    var membership="online"
    func setMembership(_ value:String) {membership=value}
    func connect(endpoint:String,prefix:String) async throws {connected=true}
    func disconnect() async {connected=false}
    func snapshot() async -> FleetSnapshot {FleetSnapshot(link:connected ? "healthy":"disconnected",fleetAge:connected ? 0:nil,rovers:connected ? [RoverView(id:8,membership:membership,pose:nil,poseAge:nil,goal:nil,goalAge:nil,commandPhase:"none",commandToken:nil)]:[],notice:nil)}
    func send(id:UInt64,waypoint:Waypoint) async throws -> String {sendCount+=1;return "test-token"}
    func cancel(id:UInt64) async throws {}
}
