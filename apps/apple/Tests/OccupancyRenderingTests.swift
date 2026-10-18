import XCTest
import SwiftUI
import MapKit
#if canImport(ARGOS)
@testable import ARGOS
#endif

final class OccupancyRenderingTests: XCTestCase {
  private func map() -> OccupancyFrame {
    OccupancyFrame(schemaVersion: 1, roverID: 8, runID: "run", sequence: 1,
      width: 2, height: 2, resolution: 0.25, originX: -3, originY: 7,
      occupancy: [-1, 35, 36, 65], age: 0)
  }

  func testRedrawAndFreshnessReuseContentWithoutRasterizing() {
    let cache = OccupancyRasterCache()
    var frame = map()
    let first = cache.image(for: frame)
    XCTAssertNotNil(first)
    for _ in 0..<1000 { XCTAssertTrue(cache.image(for: frame) === first) }
    frame.age = 0.49
    XCTAssertTrue(cache.image(for: frame) === first)
    frame.age = 0.5
    XCTAssertNil(cache.image(for: frame))
    frame.age = 0
    XCTAssertTrue(cache.image(for: frame) === first)
    XCTAssertEqual(cache.rasterizationCount, 1)
  }

  func testIdentityGeometrySelectionAndSessionInvalidation() {
    let cache = OccupancyRasterCache()
    var frame = map()
    let first = cache.image(for: frame)
    frame.roverID = 9
    XCTAssertNotNil(cache.image(for: frame))
    frame.roverID = 8
    XCTAssertTrue(cache.image(for: frame) === first)
    frame.sequence += 1
    XCTAssertTrue(cache.image(for: frame) === first)
    frame.occupancy[0] = 0
    XCTAssertNotNil(cache.image(for: frame))
    frame.runID = "new-run"
    XCTAssertNotNil(cache.image(for: frame))
    frame.resolution *= 2
    XCTAssertNotNil(cache.image(for: frame))
    frame.originX += 1
    XCTAssertNotNil(cache.image(for: frame))
    XCTAssertEqual(cache.rasterizationCount, 6)
    cache.reset()
    XCTAssertNotNil(cache.image(for: frame))
    XCTAssertEqual(cache.rasterizationCount, 7)
  }

  func testNewRecoveryRevisionWithIdenticalCellsRetainsTexture() {
    let cache = OccupancyRasterCache()
    var frame = map()
    let first = cache.image(for: frame)
    for sequence in 2...100 {
      frame.sequence = UInt64(sequence)
      XCTAssertTrue(cache.image(for: frame) === first)
    }
    XCTAssertEqual(cache.rasterizationCount, 1)
  }

  func testCacheEvictsLeastRecentlyUsedTexturesWithinByteBudget() {
    let cache = OccupancyRasterCache(maximumBytes: 576)
    var frame = map()
    let first = cache.image(for: frame)
    frame.roverID = 9
    XCTAssertNotNil(cache.image(for: frame))
    frame.roverID = 8
    XCTAssertTrue(cache.image(for: frame) === first)
    frame.roverID = 10
    XCTAssertNotNil(cache.image(for: frame))
    XCTAssertEqual(cache.cachedBytes, 576)
    frame.roverID = 8
    XCTAssertTrue(cache.image(for: frame) === first)
    XCTAssertEqual(cache.rasterizationCount, 3)
    frame.roverID = 9
    XCTAssertNotNil(cache.image(for: frame))
    XCTAssertEqual(cache.rasterizationCount, 4)
    XCTAssertEqual(cache.cachedBytes, 576)
    cache.reset()
    XCTAssertEqual(cache.cachedBytes, 0)
  }

  func testInvalidContentAndSemanticThresholds() {
    let cache = OccupancyRasterCache()
    var frame = map()
    frame.occupancy = [0]
    XCTAssertNil(cache.image(for: frame))
    frame = map(); frame.occupancy[0] = 101
    XCTAssertNil(cache.image(for: frame))
    XCTAssertEqual(cache.rasterizationCount, 0)
    XCTAssertEqual(OccupancyRasterCache.rgba(value: -1).3, 6)
    XCTAssertEqual(OccupancyRasterCache.rgba(value: -1, boundary: true).3, 31)
    XCTAssertEqual(OccupancyRasterCache.rgba(value: 0).3, 18)
    XCTAssertEqual(OccupancyRasterCache.rgba(value: 35).3, 18)
    XCTAssertEqual(OccupancyRasterCache.rgba(value: 36).3, 51)
    XCTAssertEqual(OccupancyRasterCache.rgba(value: 64).3, 51)
    XCTAssertEqual(OccupancyRasterCache.rgba(value: 65).3, 128)
    XCTAssertEqual(OccupancyRasterCache.rgba(value: 100).3, 128)
    let image = cache.image(for: map())!
    let data = image.dataProvider!.data!
    let bytes = CFDataGetBytePtr(data)!
    // Row zero stays local y=originY; affine placement, not a row flip, controls screen orientation.
    XCTAssertEqual(bytes[(1 * image.width + 1) * 4 + 3], 6)
    XCTAssertEqual(bytes[(1 * image.width + 5) * 4 + 3], 18)
    XCTAssertEqual(bytes[(5 * image.width + 1) * 4 + 3], 51)
    XCTAssertEqual(bytes[(5 * image.width + 5) * 4 + 3], 128)
  }

  func testMalformedSameIdentityDoesNotReplayCachedTexture() {
    let cache = OccupancyRasterCache()
    var frame = map()
    let first = cache.image(for: frame)
    frame.occupancy = [0]
    XCTAssertNil(cache.image(for: frame))
    frame = map(); frame.occupancy[0] = 101
    XCTAssertNil(cache.image(for: frame))
    frame = map(); frame.occupancy[0] = 0
    XCTAssertFalse(cache.image(for: frame) === first)
    XCTAssertEqual(cache.rasterizationCount, 2)
  }

  func testAffineRasterMatchesEveryCellUnderPanZoomOrbitAndGeographicPlacement() {
    let size = CGSize(width: 900, height: 600)
    let origin = CLLocationCoordinate2D(latitude: 47, longitude: -122)
    let ref = GeographicReference(version: 1, frameID: "geo", mode: "geographic",
      reason: "", tracking: "normal", age: 0, gpsAccuracy: 1, headingAccuracy: 1,
      originLatitude: 47.001, originLongitude: -122.001,
      anchorX: 3, anchorY: 4, rotation: 0.7)
    let frame = map()
    for geographic in [false, true] {
      for dimensional in [false, true] {
        for azimuth in [0.0, 0.7, 2.3] {
          let camera = FieldCamera(x: 20, y: -10, extent: 8,
            dimensional: dimensional, azimuth: azimuth)
          func project(_ x: Double, _ y: Double) -> CGPoint {
            let point = geographic ? ref.scene(x: x, y: y, origin: origin) : (x: x, y: y)
            return camera.screen(x: point.x, y: point.y, size: size)
          }
          let transform = OccupancyRasterCache.transform(
            origin: project(frame.originX, frame.originY),
            xCorner: project(frame.originX + 0.5, frame.originY),
            yCorner: project(frame.originX, frame.originY + 0.5), width: 8, height: 8)
          for row in 0...2 {
            for col in 0...2 {
              let actual = CGPoint(x: col * 4, y: row * 4).applying(transform)
              let expected = project(frame.originX + Double(col) * 0.25,
                frame.originY + Double(row) * 0.25)
              XCTAssertEqual(actual.x, expected.x, accuracy: 0.00001)
              XCTAssertEqual(actual.y, expected.y, accuracy: 0.00001)
            }
          }
        }
      }
    }
  }

  func testOffscreenQuadCullIncludesFourthCornerAndCrossingQuads() {
    let size = CGSize(width: 100, height: 100)
    XCTAssertFalse(OccupancyRasterCache.intersectsViewport(origin: CGPoint(x: 200, y: 200),
      xCorner: CGPoint(x: 210, y: 200), yCorner: CGPoint(x: 200, y: 210), size: size))
    // Three corners outside, fourth inside.
    XCTAssertTrue(OccupancyRasterCache.intersectsViewport(origin: CGPoint(x: -100, y: -100),
      xCorner: CGPoint(x: 50, y: -100), yCorner: CGPoint(x: -100, y: 50), size: size))
    XCTAssertTrue(OccupancyRasterCache.intersectsViewport(origin: CGPoint(x: -100, y: -100),
      xCorner: CGPoint(x: 200, y: -100), yCorner: CGPoint(x: -100, y: 200), size: size))
  }
}
