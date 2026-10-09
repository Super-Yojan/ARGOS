import XCTest
final class OperatorFlowTests:XCTestCase {
    func testLiveWaypointAndCancel() throws {
        let app=XCUIApplication()
        app.launchArguments=["--connect","-endpoint","tcp/127.0.0.1:7447","-prefix","terra/rover"]
        app.launch()
        let rover=app.buttons["scene-rover-0"]
        XCTAssertTrue(rover.waitForExistence(timeout:15));rover.tap()
        let scene=app.otherElements["field-scene"]
        XCTAssertTrue(scene.waitForExistence(timeout:5))
        scene.coordinate(withNormalizedOffset:CGVector(dx:0.22,dy:0.65)).tap()
        let send=app.buttons["send-waypoint"]
        #if os(iOS)
        if !send.isHittable {app.swipeUp()}
        #endif
        XCTAssertTrue(send.isEnabled);send.tap()
        let arrived=NSPredicate(format:"label CONTAINS 'arrived'")
        expectation(for:arrived,evaluatedWith:app.staticTexts["command-phase"])
        waitForExpectations(timeout:45)
        app.buttons["cancel-goal"].tap()
        let cancelled=NSPredicate(format:"label CONTAINS 'cancelled'")
        expectation(for:cancelled,evaluatedWith:app.staticTexts["command-phase"])
        waitForExpectations(timeout:8)
        let shot=XCTAttachment(screenshot:app.screenshot());shot.name="ARGOS native operator dashboard";shot.lifetime = .keepAlways;add(shot)
    }
}
