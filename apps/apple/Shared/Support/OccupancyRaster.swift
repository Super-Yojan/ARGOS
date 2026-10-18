import SwiftUI

/// Content identity deliberately excludes age, camera and geographic placement.
struct OccupancyRasterKey: Hashable {
  let rover: UInt64
  let run: String
  let sequence: UInt64
  let width: Int
  let height: Int
  let resolution: Double
  let originX: Double
  let originY: Double

  init(_ map: OccupancyFrame) {
    rover = map.roverID; run = map.runID; sequence = map.sequence
    width = map.width; height = map.height; resolution = map.resolution
    originX = map.originX; originY = map.originY
  }

  func sameGeometry(as other: OccupancyRasterKey) -> Bool {
    rover == other.rover && run == other.run && width == other.width && height == other.height
      && resolution == other.resolution && originX == other.originX && originY == other.originY
  }
}

/// Latest textures are reused across selection changes, within a fixed byte budget.
final class OccupancyRasterCache {
  private struct Entry {
    var key: OccupancyRasterKey
    let image: CGImage
    var occupancy: [Int]
    var lastUsed: UInt64
    var bytes: Int { image.bytesPerRow * image.height + occupancy.count * MemoryLayout<Int>.stride }
  }
  private var entries: [UInt64: Entry] = [:]
  private let maximumBytes: Int
  private var access: UInt64 = 0
  private(set) var cachedBytes = 0
  private(set) var rasterizationCount = 0
  private static let cyan = resolvedRGB(.cyan)
  private static let orange = resolvedRGB(.orange)

  init(maximumBytes: Int = 32 * 1024 * 1024) {
    self.maximumBytes = max(0, maximumBytes)
  }

  private static func resolvedRGB(_ color: Color) -> (UInt8, UInt8, UInt8) {
    var environment = EnvironmentValues()
    environment.colorScheme = .dark
    let color = color.resolve(in: environment)
    return (UInt8((color.red * 255).rounded()), UInt8((color.green * 255).rounded()),
      UInt8((color.blue * 255).rounded()))
  }

  func image(for map: OccupancyFrame) -> CGImage? {
    guard map.schemaVersion == 1, map.age.isFinite, map.age >= 0, map.age < 0.5 else { return nil }
    let key = OccupancyRasterKey(map)
    access &+= 1
    if var entry = entries[map.roverID], entry.key.sameGeometry(as: key),
      entry.occupancy == map.occupancy {
      // Array equality is constant-time for shared storage. Rebind equal newly
      // decoded arrays so subsequent gestures reuse that fast path. Changed or
      // malformed same-sequence content must pass validation below, not replay
      // a previously valid texture.
      // A recovery baseline may advance sequence without changing any cells.
      entry.key = key
      entry.occupancy = map.occupancy
      entry.lastUsed = access
      entries[map.roverID] = entry
      return entry.image
    }
    guard map.usable else { return nil }
    // Four texels per cell retain a subtle dotted unknown-cell boundary.
    let side = 4
    let width = map.width * side, height = map.height * side
    var bytes = [UInt8](repeating: 0, count: width * height * 4)
    for row in 0..<map.height {
      for col in 0..<map.width {
        let value = map.occupancy[row * map.width + col]
        for y in 0..<side {
          for x in 0..<side {
            let rgba = Self.rgba(value: value, boundary: x == 0 && y == 0)
            let offset = ((row * side + y) * width + col * side + x) * 4
            bytes[offset] = rgba.0; bytes[offset + 1] = rgba.1
            bytes[offset + 2] = rgba.2; bytes[offset + 3] = rgba.3
          }
        }
      }
    }
    guard let provider = CGDataProvider(data: Data(bytes) as CFData),
      let image = CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32,
        bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
        provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    else { return nil }
    rasterizationCount += 1
    if let previous = entries.removeValue(forKey: map.roverID) { cachedBytes -= previous.bytes }
    let bytesUsed = image.bytesPerRow * image.height + map.occupancy.count * MemoryLayout<Int>.stride
    if bytesUsed <= maximumBytes {
      while cachedBytes + bytesUsed > maximumBytes,
        let oldest = entries.min(by: { $0.value.lastUsed < $1.value.lastUsed })
      {
        cachedBytes -= oldest.value.bytes
        entries.removeValue(forKey: oldest.key)
      }
      entries[map.roverID] = Entry(key: key, image: image, occupancy: map.occupancy, lastUsed: access)
      cachedBytes += bytesUsed
    }
    return image
  }

  static func rgba(value: Int, boundary: Bool = false) -> (UInt8, UInt8, UInt8, UInt8) {
    if value < 0 { return (255, 255, 255, boundary ? 31 : 6) }
    if value >= 65 { return (cyan.0, cyan.1, cyan.2, 128) }
    if value <= 35 { return (cyan.0, cyan.1, cyan.2, 18) }
    return (orange.0, orange.1, orange.2, 51)
  }

  func reset() {
    entries.removeAll()
    cachedBytes = 0
    access = 0
  }

  static func intersectsViewport(origin: CGPoint, xCorner: CGPoint, yCorner: CGPoint,
    size: CGSize) -> Bool {
    let fourth = CGPoint(x: xCorner.x + yCorner.x - origin.x,
      y: xCorner.y + yCorner.y - origin.y)
    let corners = [origin, xCorner, yCorner, fourth]
    guard corners.allSatisfy({ $0.x.isFinite && $0.y.isFinite }) else { return false }
    return corners.map(\.x).max()! >= 0 && corners.map(\.x).min()! <= size.width
      && corners.map(\.y).max()! >= 0 && corners.map(\.y).min()! <= size.height
  }

  /// Image coordinates increase with local x/y, not screen x/y. Three projected
  /// corners define the affine ground plane, including orbit and geographic rotation.
  static func transform(origin: CGPoint, xCorner: CGPoint, yCorner: CGPoint,
    width: Int, height: Int) -> CGAffineTransform {
    CGAffineTransform(a: (xCorner.x - origin.x) / Double(width),
      b: (xCorner.y - origin.y) / Double(width),
      c: (yCorner.x - origin.x) / Double(height),
      d: (yCorner.y - origin.y) / Double(height), tx: origin.x, ty: origin.y)
  }
}
