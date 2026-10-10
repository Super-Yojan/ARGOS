import XCTest
#if canImport(ARGOS)
@testable import ARGOS
#endif
final class FleetModelTests: XCTestCase {
    @MainActor func testDirectWaypointPreservesL2WithDelayedTelemetry() async {
        let backend = DriveTestBackend()
        let model = FleetModel(backend: backend)
        await model.connect(endpoint: "test", prefix: "test"); model.stopObservation()
        await backend.directWaypoint(); await model.refresh()
        let sent = await model.send(id: 8, waypoint: .local(x: 5, y: 6, yaw: nil))
        XCTAssertTrue(sent)
        let level = await backend.level; XCTAssertEqual(level, "waypoint_direct")
        let takeovers = await backend.takeovers; XCTAssertEqual(takeovers, 0)
        let count = await backend.waypointsSent; XCTAssertEqual(count, 1)
        await model.disconnect()
    }

    @MainActor func testDiscardBeforeDelayedWaypointModeAckNeverDispatches() async {
        let backend = DriveTestBackend()
        let model = FleetModel(backend: backend)
        await model.connect(endpoint: "test", prefix: "test"); model.stopObservation()
        await backend.delayFirstTakeover()
        let send = Task { await model.send(id: 8, waypoint: .local(x: 5, y: 6, yaw: nil)) }
        for _ in 0..<100 { if model.pendingWaypointID != nil { break }; try? await Task.sleep(for: .milliseconds(5)) }
        XCTAssertEqual(model.pendingWaypointID, 8)
        model.cancelPendingWaypoint()
        let sent = await send.value; XCTAssertFalse(sent)
        let count = await backend.waypointsSent; XCTAssertEqual(count, 0)
        XCTAssertFalse(model.busy)
        await model.disconnect()
    }
    @MainActor func testForestDelayAllowsWaypointButNeverManualDrive() async {
        let backend = DriveTestBackend()
        let model = FleetModel(backend: backend)
        await model.connect(endpoint: "test", prefix: "test"); model.stopObservation()
        await backend.forestDelay(age: 60); await model.refresh()
        XCTAssertTrue(model.waypointAvailable(8)); XCTAssertFalse(model.driveAvailable(8)); XCTAssertFalse(model.motorsArmed(8))
        await model.send(id: 8, waypoint: .local(x: 5, y: 6, yaw: nil))
        let sent = await backend.waypointsSent; XCTAssertEqual(sent, 1)
        await backend.forestDelay(age: 90); await model.refresh()
        XCTAssertFalse(model.waypointAvailable(8))
        await model.disconnect()
    }
    @MainActor func testDelayedRefreshCannotRestorePreviousSession() async {
        let backend = SessionSwitchBackend()
        let model = FleetModel(backend: backend)
        await model.connect(endpoint: "old", prefix: "test"); model.stopObservation()
        await backend.delayNextSnapshot()
        let old = Task { await model.refresh() }
        for _ in 0..<100 { if await backend.waiting { break }; try? await Task.sleep(for: .milliseconds(5)) }
        let waiting = await backend.waiting; XCTAssertTrue(waiting)
        let epoch = model.sessionEpoch
        await model.connect(endpoint: "new", prefix: "test"); model.stopObservation()
        await backend.releaseSnapshot(); await old.value
        XCTAssertGreaterThan(model.sessionEpoch, epoch)
        XCTAssertEqual(model.sceneRovers.map(\.id), [2])
        XCTAssertEqual(model.selected, 2)
        await model.disconnect()
    }
    func testSceneDraftInvalidation() {
        var state = FleetSceneState()
        state.select(8); state.draft = ScenePoint(x: 1, y: 2); state.inspecting = true
        state.select(9); XCTAssertNil(state.draft); XCTAssertFalse(state.inspecting)
        state.draft = ScenePoint(x: 3, y: 4)
        state.reconcile(frame: "new-session", ids: [9])
        XCTAssertNil(state.selectedID); XCTAssertNil(state.draft)
        state.select(9); state.reconcile(frame: "new-session", ids: [])
        XCTAssertNil(state.selectedID)
    }
    func testOrbitProjectionRoundTrip() {
        for angle in [0.0, 0.5, 1.5, -2.0] {
            var camera = FieldCamera(); camera.azimuth = angle
            let point = camera.screen(x: 12, y: -7, size: CGSize(width: 1200, height: 800))
            let world = camera.world(point, size: CGSize(width: 1200, height: 800))
            XCTAssertEqual(world.x, 12, accuracy: 0.00001); XCTAssertEqual(world.y, -7, accuracy: 0.00001)
        }
    }
    @MainActor func testDelayedOldTakeoverCannotDriveNewSelection() async {
        let backend = DriveTestBackend(); await backend.delayFirstTakeover()
        let model = FleetModel(backend: backend)
        await model.connect(endpoint: "tcp/127.0.0.1:7448", prefix: "test")
        let drive = RoverDriveController(); drive.attach(model)
        let old = Task { await drive.start(id:8) }
        try? await Task.sleep(for:.milliseconds(30))
        drive.stop()
        await drive.start(id:9); await model.refresh()
        try? await Task.sleep(for:.milliseconds(150)); drive.joystick(CGPoint(x:0,y:-1))
        await old.value
        try? await Task.sleep(for:.milliseconds(150))
        let movingIDs = await backend.movingIDs
        XCTAssertFalse(movingIDs.contains(8))
        drive.detach(); await model.disconnect()
    }
    @MainActor func testLocalFrameResetChangesFrameKeyAndRejectsOldCloud() async {
        let backend = DriveTestBackend()
        let model = FleetModel(backend:backend)
        await model.connect(endpoint:"tcp/127.0.0.1:7448",prefix:"test")
        let initial = model.frameKey
        XCTAssertNotNil(model.sceneCloud(8))
        await backend.resetLocalFrame(); await model.refresh()
        XCTAssertNotEqual(model.frameKey,initial)
        XCTAssertNil(model.sceneCloud(8))
        await model.disconnect()
    }
    @MainActor func testManualDrivePublishesNeutralOnReleaseAndTelemetryLoss() async {
        let backend = DriveTestBackend()
        let model = FleetModel(backend: backend)
        await model.connect(endpoint: "tcp/127.0.0.1:7448", prefix: "test")
        let drive = RoverDriveController(); drive.attach(model)
        await drive.start(id: 8)
        await model.refresh()
        try? await Task.sleep(for: .milliseconds(150))
        XCTAssertTrue(drive.enabled)
        drive.joystick(CGPoint(x: 0, y: -1))
        try? await Task.sleep(for: .milliseconds(150))
        let moving = await backend.vectors
        XCTAssertTrue(moving.contains { $0.linear > 0 })
        drive.joystick(nil)
        try? await Task.sleep(for: .milliseconds(150))
        let released = await backend.vectors
        XCTAssertEqual(released.last, .zero)
        drive.joystick(CGPoint(x: 0, y: -1))
        await backend.loseTelemetry(); await model.refresh()
        try? await Task.sleep(for: .milliseconds(150))
        XCTAssertFalse(drive.enabled)
        let stopped = await backend.vectors
        XCTAssertEqual(stopped.last, .zero)
        drive.detach(); await model.disconnect()
    }
    func testOccupancySnapshotUsesCellCentersAndNeverTreatsUnknownAsFree() {
        var grid = OccupancyFrame(schemaVersion: 1, roverID: 8, runID: "run", sequence: 1, width: 2, height: 2, resolution: 0.5, originX: 3, originY: -1, occupancy: [-1,0,100,50], age: 0)
        XCTAssertTrue(grid.usable)
        XCTAssertEqual(grid.value(x: 3.25, y: -0.75), -1)
        XCTAssertEqual(grid.value(x: 3.75, y: -0.75), 0)
        XCTAssertEqual(grid.value(x: 3.25, y: -0.25), 100)
        XCTAssertNil(grid.value(x: 2, y: 0))
        grid.age = 0.6; XCTAssertFalse(grid.usable)
        grid.age = 0; grid.occupancy = [0]; XCTAssertFalse(grid.usable)
    }
    func testDriveInputsHaveDeadzoneBoundsAndExclusiveSources() {
        XCTAssertEqual(DriveVector.axes(forward: 0.05, turn: 0.05), .zero)
        let drive = DriveVector.axes(forward: 3, turn: -2)
        XCTAssertEqual(drive.linear, 0.5)
        XCTAssertEqual(drive.angular, -1)
        XCTAssertEqual(DriveVector.keyboard(["w", "s", "a", "d"]), .zero)
        XCTAssertGreaterThan(DriveVector.keyboard(["w", "a"]).angular, 0)
    }
    func testPointCloudRejectsStaleOversizedAndInvalidCoordinates() throws {
        let json = #"{"version":1,"sequence":4,"frameID":"test","source":"features","age":0.1,"points":[[1,2,3],[4,5,6]]}"#
        var cloud = try JSONDecoder().decode(PointCloudFrame.self, from: Data(json.utf8))
        XCTAssertTrue(cloud.usable)
        cloud.age = 2.5; XCTAssertFalse(cloud.usable)
        cloud.age = 0; cloud.points = [[.nan, 0, 0]]; XCTAssertFalse(cloud.usable)
        cloud.points = Array(repeating: [0,0,0], count: 4097); XCTAssertFalse(cloud.usable)
    }
    func testAutonomyRequiresFreshReportedSupportedLevelAndAcknowledgement() throws {
        var authority = VehicleAuthority(runID: "run", requestedLevel: "teleop", effectiveLevel: "teleop", activeSource: "operator", safety: "active", reason: "ready", revision: 4, token: "takeover", result: "accepted", supportedLevels: ["teleop", "waypoint"], age: 0)
        XCTAssertTrue(authority.fresh)
        XCTAssertTrue(authority.acceptsDrive(token: "takeover"))
        XCTAssertFalse(authority.acceptsDrive(token: "other"))
        authority.age = 0.6; XCTAssertFalse(authority.fresh)
        authority.age = 0; authority.safety = "stopped"; XCTAssertFalse(authority.acceptsDrive(token: "takeover"))
    }
    func testGeographicAlignmentRoundTripAndStaleness() {
        var ref = GeographicReference(version: 1, frameID: "frame", mode: "geographic", reason: "Reliable", tracking: "normal", age: 0.2,
            gpsAccuracy: 4, headingAccuracy: 5, originLatitude: 38, originLongitude: -77, anchorX: 3, anchorY: -2, rotation: -.pi/2)
        XCTAssertTrue(ref.usable)
        let origin = ref.origin!
        let scene = ref.scene(x: 8, y: 4, origin: origin)
        XCTAssertEqual(scene.x, 6, accuracy: 0.001)
        XCTAssertEqual(scene.y, -5, accuracy: 0.001)
        let local = ref.local(x: scene.x, y: scene.y, origin: origin)
        XCTAssertEqual(local.x, 8, accuracy: 0.001)
        XCTAssertEqual(local.y, 4, accuracy: 0.001)
        ref.age = 90; XCTAssertFalse(ref.usable)
        ref.age = 0; ref.mode = "local"; XCTAssertFalse(ref.usable)
        ref.mode = "geographic"; ref.rotation = .nan; XCTAssertFalse(ref.usable)
    }
    @MainActor func testUnalignedGoalIsNotPlacedInGeographicScene() async {
        let model = FleetModel(backend: MixedPositioningBackend())
        await model.refresh()
        XCTAssertNotNil(model.geographicOrigin)
        XCTAssertTrue(model.canPlaceGeographically(8))
        XCTAssertFalse(model.canPlaceGeographically(9))
        XCTAssertEqual(model.sceneRovers.first { $0.id == 9 }?.goal?.state, "active")
    }
    @MainActor func testRoverFootprintStaysUnder35Inches() {
        guard let bounds = RoverSceneController.template?.boundingBox else { return XCTFail("Missing CAD model") }
        let scale = RoverSceneController.physicalScale
        XCTAssertLessThan(Double(bounds.max.x - bounds.min.x) * Double(scale), 35 * 0.0254)
        XCTAssertLessThan(Double(bounds.max.z - bounds.min.z) * Double(scale), 35 * 0.0254)
    }
    @MainActor func testRealRoverAssetIsBundled() {
        XCTAssertNotNil(RoverSceneController.template)
        let bounds = RoverSceneController.template?.boundingBox
        XCTAssertGreaterThan(bounds?.max.x ?? 0, bounds?.min.x ?? 0)
    }
    func testUnknownVehicleMarkersDoNotOverlapOnPhone() {
        let desired = (0..<6).map { (UInt64($0), CGPoint(x: 195, y: 650)) }
        let points = FieldMarkerPlacement.locations(desired: desired, size: CGSize(width: 390, height: 650))
        XCTAssertEqual(points.count, 6)
        for id in 0..<6 {
            for other in (id + 1)..<6 {
                XCTAssertNotEqual(points[UInt64(id)], points[UInt64(other)])
            }
        }
    }
    func testSceneCameraKeepsCoordinatesCorrectInBothViews() {
        let size = CGSize(width: 1100, height: 650)
        for dimensional in [false, true] {
            let camera = FieldCamera(x: 11, y: -4, extent: 30, dimensional: dimensional)
            let screen = camera.screen(x: 17, y: 9, size: size)
            let world = camera.world(screen, size: size)
            XCTAssertEqual(world.x, 17, accuracy: 0.000001)
            XCTAssertEqual(world.y, 9, accuracy: 0.000001)
            let elevated = camera.screen(x: 17, y: 9, z: 2, size: size)
            XCTAssertEqual(elevated.x, screen.x)
            XCTAssertLessThan(elevated.y, screen.y)
        }
    }
    func testMissingAndStaleTelemetryNeedsAttention() {
        func rover(poseAge: Double?, goalAge: Double?, phase: String = "none") -> RoverView {
            RoverView(id: 8, membership: "online", pose: nil, poseAge: poseAge,
                goal: GoalProgress(state: "active", goalId: 1, token: "goal", distance: 2, x: 2, y: 0),
                goalAge: goalAge, commandPhase: phase, commandToken: nil)
        }
        XCTAssertEqual(DashboardPresentation.attentionReason(rover(poseAge: nil, goalAge: 0)), "Position unavailable")
        XCTAssertEqual(DashboardPresentation.attentionReason(rover(poseAge: 90, goalAge: 0)), "Position is stale")
        XCTAssertEqual(DashboardPresentation.attentionReason(rover(poseAge: 0, goalAge: 90)), "Goal status is stale")
        XCTAssertEqual(DashboardPresentation.attentionReason(rover(poseAge: 0, goalAge: 0, phase: "unconfirmed")), "Command unconfirmed")
        XCTAssertNil(DashboardPresentation.attentionReason(rover(poseAge: 0, goalAge: 0)))
    }
    @MainActor func testOfflineFleetRemainsSelectableWithoutAList() async {
        let backend = FakeBackend()
        await backend.setMembership("stale")
        let model = FleetModel(backend: backend)
        await model.connect(endpoint: "tcp/127.0.0.1:7447", prefix: "terra/rover")
        XCTAssertEqual(model.selected, 8)
        XCTAssertEqual(DashboardPresentation.attentionReason(model.snapshot.rovers[0]), "Vehicle stale")
        await model.disconnect()
    }
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
    @MainActor func testDashboardArmUsesConfirmedStateAndDisarmSurvivesAuthorityLoss() async {
        let backend = DriveTestBackend()
        await backend.setHardwareArmed(false)
        let model = FleetModel(backend: backend)
        await model.connect(endpoint: "tcp/127.0.0.1:7448", prefix: "terra/phone")
        XCTAssertFalse(model.motorsArmed(8))
        await model.requestHardware(id: 8, arm: true)
        XCTAssertFalse(model.motorsArmed(8))
        await model.refresh()
        XCTAssertTrue(model.motorsArmed(8))
        await backend.hideAuthority()
        await model.refresh()
        XCTAssertNil(model.authorities[8])
        await model.requestHardware(id: 8, arm: false)
        XCTAssertNil(model.error)
        await model.refresh()
        XCTAssertFalse(model.motorsArmed(8))
        let actions = await backend.hardwareActions
        XCTAssertEqual(actions, ["arm", "disarm"])
        await model.disconnect()
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
    func operatorTelemetry() async -> String { #"{"8":{"hardware":{"ready":true,"armed":true,"arming":false,"reason":"Armed","age":0},"authority":{"run_id":"test","requested_level":"waypoint","effective_level":"waypoint","active_source":"none","safety":"clear","reason":"ready","revision":1,"result":"accepted","supported_levels":["teleop","waypoint"],"age":0}}}"# }
    var sendCount=0
    var connected=false
    var membership="online"
    func setMembership(_ value:String) {membership=value}
    func connect(endpoint:String,prefix:String) async throws {connected=true}
    func disconnect() async {connected=false}
    func snapshot() async -> FleetSnapshot {FleetSnapshot(link:connected ? "healthy":"disconnected",fleetAge:connected ? 0:nil,rovers:connected ? [RoverView(id:8,membership:membership,pose:RoverPose(x:0,y:0,yaw:0),poseAge:0,goal:nil,goalAge:nil,commandPhase:"none",commandToken:nil)]:[],notice:nil)}
    func send(id:UInt64,waypoint:Waypoint) async throws -> String {sendCount+=1;return "test-token"}
    func cancel(id:UInt64) async throws {}
}

actor MixedPositioningBackend: FleetBackend {
    func connect(endpoint: String, prefix: String) async throws {}
    func disconnect() async {}
    func send(id: UInt64, waypoint: Waypoint) async throws -> String { "unused" }
    func cancel(id: UInt64) async throws {}
    func localization() async -> String {
        #"{"8":{"version":1,"frameID":"test","mode":"geographic","reason":"Reliable","tracking":"normal","age":0,"gpsAccuracy":4,"headingAccuracy":5,"originLatitude":38,"originLongitude":-77,"anchorX":0,"anchorY":0,"rotation":0}}"#
    }
    func snapshot() async -> FleetSnapshot {
        FleetSnapshot(link: "healthy", fleetAge: 0, rovers: [
            RoverView(id: 8, membership: "online", pose: RoverPose(x: 0, y: 0, yaw: 0), poseAge: 0, goal: nil, goalAge: nil, commandPhase: "none", commandToken: nil),
            RoverView(id: 9, membership: "online", pose: nil, poseAge: nil, goal: GoalProgress(state: "active", goalId: 1, token: nil, distance: 5, x: 5, y: 5), goalAge: 0, commandPhase: "none", commandToken: nil)
        ], notice: nil)
    }
}

actor DriveTestBackend: FleetBackend {
    var reportAge = 0.0
    var level = "teleop"
    var waypointsSent = 0
    func directWaypoint() { reportAge = 60; level = "waypoint_direct" }
    func forestDelay(age: Double) { reportAge = age; level = "waypoint" }
    var movingIDs: [UInt64] = []
    var slowFirst = false
    var takeovers = 0
    var frame = "local-A"
    func resetLocalFrame() { frame = "local-B" }
    func delayFirstTakeover() { slowFirst = true }
    func localization() async -> String {
        let report: [String:Any] = ["version":1,"frameID":frame,"mode":"local","reason":"test","tracking":"normal","age":0]
        return String(decoding:try! JSONSerialization.data(withJSONObject:["8":report,"9":report]),as:UTF8.self)
    }
    var hardwareArmed = true
    var reportAuthority = true
    var hardwareActions: [String] = []
    func setHardwareArmed(_ value: Bool) { hardwareArmed = value }
    func hideAuthority() { reportAuthority = false }
    var vectors: [DriveVector] = []
    var connected = false
    var token = "initial"
    var revision: UInt64 = 1
    func loseTelemetry() { connected = false }
    func connect(endpoint: String, prefix: String) async throws { connected = true }
    func disconnect() async { connected = false }
    func send(id: UInt64, waypoint: Waypoint) async throws -> String { waypointsSent += 1; return "unused" }
    func cancel(id: UInt64) async throws {}
    func snapshot() async -> FleetSnapshot {
        FleetSnapshot(link: connected ? "healthy" : "stale", fleetAge: 0, rovers: [RoverView(id: 8, membership: connected ? "online" : "stale", pose: RoverPose(x: 0, y: 0, yaw: 0), poseAge: connected ? reportAge : 5, goal: nil, goalAge: nil, commandPhase: "none", commandToken: nil), RoverView(id:9,membership:connected ? "online":"stale",pose:RoverPose(x:2,y:0,yaw:0),poseAge:connected ? reportAge:5,goal:nil,goalAge:nil,commandPhase:"none",commandToken:nil)], notice: nil)
    }
    func operatorTelemetry() async -> String {
        let authority: [String: Any] = ["run_id":"test","requested_level":level,"effective_level":level,"active_source":"operator","safety":"clear","reason":"ready","revision":revision,"token":token,"result":"accepted","supported_levels":["teleop","assisted_teleop","waypoint","waypoint_direct"],"age":connected ? reportAge : 1]
        let cloud: [String:Any] = ["version":1,"frameID":"local-A","sequence":1,"source":"features","age":0,"points":[[0,0,0]]]
        let hardware: [String: Any] = ["ready":true,"armed":hardwareArmed,"arming":false,"reason":hardwareArmed ? "Armed" : "Disarmed","age":connected ? reportAge : 1]
        var first: [String: Any] = ["cloud":cloud,"hardware":hardware]
        var second: [String: Any] = ["hardware":hardware]
        if reportAuthority { first["authority"] = authority; second["authority"] = authority }
        let data = try! JSONSerialization.data(withJSONObject: ["8":first,"9":second])
        return String(decoding:data,as:UTF8.self)
    }
    func operatorCommand(id: UInt64, kind: String, payload: String) async throws {
        let value = try JSONSerialization.jsonObject(with:Data(payload.utf8)) as! [String:Any]
        if kind == "hardware" {
            let action = value["action"] as! String
            hardwareActions.append(action); hardwareArmed = action == "arm"
        }
        if kind == "autonomy" {
            takeovers += 1
            if slowFirst && takeovers == 1 { try? await Task.sleep(for:.milliseconds(300)) }
            token = value["token"] as! String; level = value["level"] as! String; revision += 1
        }
        if kind == "teleop" {
            let vector = DriveVector(linear:value["linear"] as! Double,angular:value["angular"] as! Double)
            vectors.append(vector); if vector != .zero { movingIDs.append(id) }
        }
    }
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

actor SessionSwitchBackend: FleetBackend {
    var id: UInt64 = 1
    var delayed = false
    var pending: CheckedContinuation<FleetSnapshot, Never>?
    var held: FleetSnapshot?
    var waiting: Bool { pending != nil }
    func delayNextSnapshot() { delayed = true }
    func releaseSnapshot() { if let held { pending?.resume(returning: held) }; pending = nil; held = nil }
    func connect(endpoint: String, prefix: String) async throws { id = endpoint == "old" ? 1 : 2 }
    func disconnect() async {}
    func snapshot() async -> FleetSnapshot {
        let snapshot = FleetSnapshot(link: "healthy", fleetAge: 0, rovers: [RoverView(id: id, membership: "online", pose: RoverPose(x: 0, y: 0, yaw: 0), poseAge: 0, goal: nil, goalAge: nil, commandPhase: "none", commandToken: nil)], notice: nil)
        if delayed { delayed = false; held = snapshot; return await withCheckedContinuation { pending = $0 } }
        return snapshot
    }
    func send(id: UInt64, waypoint: Waypoint) async throws -> String { "unused" }
    func cancel(id: UInt64) async throws {}
}
