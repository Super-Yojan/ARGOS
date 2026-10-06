import XCTest
final class OperatorFlowTests:XCTestCase {
    func testLiveWaypointAndCancel() throws {
        let app=XCUIApplication()
        app.launchArguments=["--connect","-geographic","YES","-endpoint","tcp/127.0.0.1:7447"]
        app.launch()
        #if os(iOS)
        let row=app.staticTexts["Rover 0"].firstMatch
        XCTAssertTrue(row.waitForExistence(timeout:10));row.tap()
        #endif
        let first=app.textFields["target-first"]
        XCTAssertTrue(first.waitForExistence(timeout:15))
        first.tap();first.typeText("38.82986")
        let second=app.textFields["target-second"];second.tap();second.typeText("-77.3075")
        let send=app.buttons["send-waypoint"]
        #if os(iOS)
        if !send.isHittable {app.swipeUp()}
        #endif
        XCTAssertTrue(send.isEnabled);send.tap()
        let arrived=NSPredicate(format:"label CONTAINS 'Arrived at waypoint'")
        expectation(for:arrived,evaluatedWith:app.staticTexts["command-phase"])
        waitForExpectations(timeout:45)
        app.buttons["cancel-goal"].tap()
        let cancelled=NSPredicate(format:"label CONTAINS 'Goal cancelled'")
        expectation(for:cancelled,evaluatedWith:app.staticTexts["command-phase"])
        waitForExpectations(timeout:8)
        let shot=XCTAttachment(screenshot:app.screenshot());shot.name="ARGOS native operator dashboard";shot.lifetime = .keepAlways;add(shot)
    }
}
