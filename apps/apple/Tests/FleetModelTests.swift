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

final class AutonomyStateTests:XCTestCase {
    func testSafetyAndPendingAuthorityDisableDriving() throws {
        let data=Data(#"{"status":{"assigned_level":"waypoint","requested_level":"teleop","effective_level":"teleop","active_source":"none","safety":"clear","reason":"idle","revision":1,"supported_levels":["teleop"],"paused":false},"age":0.1,"action_phase":"pending"}"#.utf8)
        var state=try JSONDecoder().decode(OperatorState.self,from:data)
        XCTAssertFalse(state.canDrive)
        state.action_phase="accepted";XCTAssertTrue(state.canDrive)
        state.age=3;XCTAssertFalse(state.canDrive)
        state.age=0;state.status?.safety="emergency_stop";XCTAssertFalse(state.canDrive)
    }
}

final class OccupancyMapTests:XCTestCase {
    func testRowMajorGridRetainsUnknownAndOccupiedCells() throws {
        let json=#"{"grid":{"schema_version":1,"rover_id":8,"run_id":"test","sequence":1,"width":2,"height":2,"resolution":0.5,"origin_x":-1,"origin_y":3,"occupancy":[-1,0,65,100]},"age":0.1}"#
        var map=try JSONDecoder().decode(OccupancyState.self,from:Data(json.utf8))
        XCTAssertTrue(map.grid.valid);XCTAssertEqual(map.grid.occupancy,[-1,0,65,100]);XCTAssertFalse(map.stale)
        map=try JSONDecoder().decode(OccupancyState.self,from:Data(json.replacingOccurrences(of:"\"age\":0.1",with:"\"age\":3").utf8))
        XCTAssertTrue(map.stale)
        let bad=try JSONDecoder().decode(OccupancyState.self,from:Data(json.replacingOccurrences(of:"\"width\":2",with:"\"width\":3").utf8))
        XCTAssertFalse(bad.grid.valid)
    }
    func testWorldCoordinatesRoundTripWithoutRectangularMapDistortion() {
        let size=CGSize(width:800,height:320)
        let a=LocalMapProjection.screen(x:4,y:6,size:size,extent:10,centerX:2,centerY:3)
        let b=LocalMapProjection.world(a,size:size,extent:10,centerX:2,centerY:3)
        XCTAssertEqual(b.0,4,accuracy:0.000001);XCTAssertEqual(b.1,6,accuracy:0.000001)
        let origin=LocalMapProjection.screen(x:2,y:3,size:size,extent:10,centerX:2,centerY:3)
        let north=LocalMapProjection.screen(x:3,y:3,size:size,extent:10,centerX:2,centerY:3)
        let west=LocalMapProjection.screen(x:2,y:4,size:size,extent:10,centerX:2,centerY:3)
        XCTAssertEqual(origin.y-north.y,origin.x-west.x,accuracy:0.000001)
    }
}
