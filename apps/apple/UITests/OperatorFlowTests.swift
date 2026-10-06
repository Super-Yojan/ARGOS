import XCTest
final class OperatorFlowTests:XCTestCase {
    #if os(macOS)
    func testLiveAuthorityAndStop() throws {
        let app=XCUIApplication()
        app.launchArguments=["--connect","-geographic","NO","-endpoint","tcp/127.0.0.1:7448"]
        app.launch()
        if !app.windows.firstMatch.waitForExistence(timeout:5) {app.typeKey("n",modifierFlags:.command)}
        let take=app.buttons["take-over"]
        XCTAssertTrue(take.waitForExistence(timeout:15))
        if app.buttons["Reset stop"].exists {app.buttons["Reset stop"].click()}
let map=app.staticTexts["occupancy-status"]
XCTAssertTrue(map.waitForExistence(timeout:15))
expectation(for:NSPredicate(format:"label CONTAINS 'Observed map'"),evaluatedWith:map)
waitForExpectations(timeout:20)
take.click()

        let authority=app.staticTexts["effective-authority"]
        expectation(for:NSPredicate(format:"label CONTAINS 'teleop'"),evaluatedWith:authority)
        waitForExpectations(timeout:10)
        let picker=app.popUpButtons["autonomy-level"]
        XCTAssertTrue(picker.waitForExistence(timeout:5));picker.click();app.menuItems["Supervised"].click()
        let approve=app.buttons["Approve"]
        XCTAssertTrue(approve.waitForExistence(timeout:20));approve.click()
        app.buttons["emergency-stop"].click()
        XCTAssertTrue(app.buttons["Reset stop"].waitForExistence(timeout:10))
        app.buttons["Export session log"].click()
        XCTAssertTrue(app.buttons["Save or share session log"].waitForExistence(timeout:10))
        let shot=XCTAttachment(screenshot:app.screenshot());shot.name="ARGOS mission authority and safety";shot.lifetime = .keepAlways;add(shot)
    }
    #endif
    func testLiveWaypointAndCancel() throws {
        let app=XCUIApplication()
        app.launchArguments=["--connect","-geographic","NO","-endpoint",ProcessInfo.processInfo.environment["ARGOS_TEST_ENDPOINT"] ?? "tcp/127.0.0.1:7447"]
        app.launch()
        #if os(iOS)
        let row=app.staticTexts["Rover 0"].firstMatch
        XCTAssertTrue(row.waitForExistence(timeout:10));row.tap()
        #endif
        #if os(macOS)
        let picker=app.popUpButtons["autonomy-level"]
        XCTAssertTrue(picker.waitForExistence(timeout:10));picker.click()
        app.menuItems["Waypoint"].click()
        #else
        let picker=app.buttons["autonomy-level"]
        XCTAssertTrue(picker.waitForExistence(timeout:10));picker.tap()
        app.buttons["Waypoint"].tap()
        #endif
        let first=app.textFields["target-first"]

        XCTAssertTrue(first.waitForExistence(timeout:15))
        first.tap();first.typeText("2.0")
        let second=app.textFields["target-second"];second.tap();second.typeText("0.0")
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
