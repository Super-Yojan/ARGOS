import SwiftUI

struct LocalMap: View {
  let rovers: [RoverView]
  let selected: UInt64
  let occupancy: OccupancyState?
  let draft: (Double, Double)?
  var select: (UInt64) -> Void = { _ in }
  let pick: (Double, Double) -> Void
  @State private var extent = 30.0
  private var centerX: Double {
    occupancy.map { $0.grid.origin_x + Double($0.grid.width) * $0.grid.resolution / 2 } ?? 0
  }
  private var centerY: Double {
    occupancy.map { $0.grid.origin_y + Double($0.grid.height) * $0.grid.resolution / 2 } ?? 0
  }
  var body: some View {
    GeometryReader { geometry in
      ZStack(alignment: .topLeading) {
        Canvas { context, size in
          if let occupancy {
            let g = occupancy.grid
            var unknown = Path()
            var free = Path()
            var occupied = Path()
            for row in 0..<g.height {
              for col in 0..<g.width {
                let a = screen(
                  x: g.origin_x + Double(col) * g.resolution,
                  y: g.origin_y + Double(row) * g.resolution, size: size)
                let b = screen(
                  x: g.origin_x + Double(col + 1) * g.resolution,
                  y: g.origin_y + Double(row + 1) * g.resolution, size: size)
                let rect = CGRect(x: b.x, y: b.y, width: a.x - b.x, height: a.y - b.y)
                guard rect.intersects(CGRect(origin: .zero, size: size)) else { continue }
                let value = g.occupancy[row * g.width + col]
                if value < 0 {
                  unknown.addRect(rect)
                } else if value >= 65 {
                  occupied.addRect(rect)
                } else {
                  free.addRect(rect)
                }
              }
            }
            context.opacity = occupancy.stale ? 0.45 : 1
            context.fill(unknown, with: .color(.gray.opacity(0.3)))
            context.fill(free, with: .color(.white))
            context.fill(occupied, with: .color(.black.opacity(0.85)))
            context.opacity = 1
          }
          for rover in rovers {
            if let pose = rover.pose {
              let point = screen(x: pose.x, y: pose.y, size: size)
              context.fill(
                Path(ellipseIn: CGRect(x: point.x - 6, y: point.y - 6, width: 12, height: 12)),
                with: .color(
                  (rover.poseAge ?? 99) < 2.5 ? (rover.id == selected ? .blue : .teal) : .gray))
              context.draw(
                Text("\(rover.id)").font(.caption), at: CGPoint(x: point.x, y: point.y + 17))
            }
            if let goal = rover.goal, goal.state != "idle" {
              context.draw(
                Text("⚑").foregroundStyle(.orange), at: screen(x: goal.x, y: goal.y, size: size))
            }
          }
          if let draft {
            context.draw(
              Text("⊕").font(.title).foregroundStyle(.purple),
              at: screen(x: draft.0, y: draft.1, size: size))
          }
        }.background(Color.secondary.opacity(0.05)).accessibilityLabel(
          "Observed occupancy map. White is free, black occupied, gray unknown."
        ).accessibilityIdentifier("occupancy-grid")
        VStack(alignment: .leading, spacing: 4) {
          Text("OCCUPANCY · ROVER \(selected)").font(.caption.weight(.semibold))
          Text("↑ +x north · ← +y west").font(.caption)
          if let occupancy {
            Text(
              occupancy.stale
                ? "Stale map · \(String(format:"%.1f",occupancy.age)) s old"
                : "Observed map · \(occupancy.grid.width) × \(occupancy.grid.height)"
            ).font(.caption).accessibilityElement(children: .ignore).accessibilityLabel(
              occupancy.stale ? "Stale map" : "Observed map"
            ).accessibilityIdentifier("occupancy-status")
          } else {
            Text("Waiting for observed map").font(.caption).accessibilityIdentifier(
              "occupancy-status")
          }
          HStack(spacing: 10) {
            legend("Free", .white)
            legend("Occupied", .black)
            legend("Unknown", .gray.opacity(0.3))
          }
        }.padding(10).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8)).padding(8)
          .allowsHitTesting(false)
        VStack {
          Spacer()
          HStack {
            Text("±\(Int(extent)) m").font(.caption)
            Spacer()
            Button("Fit map") { fit() }
            Button {
              extent = min(10000, extent * 2)
            } label: {
              Image(systemName: "minus.magnifyingglass")
            }
            Button {
              extent = max(1, extent / 2)
            } label: {
              Image(systemName: "plus.magnifyingglass")
            }
          }
        }.padding(12)
      }
      .onTapGesture { point in
        if let hit = rovers.filter({ $0.pose != nil }).min(by: { lhs, rhs in
          let a = screen(x: lhs.pose!.x, y: lhs.pose!.y, size: geometry.size)
          let b = screen(x: rhs.pose!.x, y: rhs.pose!.y, size: geometry.size)
          return hypot(a.x - point.x, a.y - point.y) < hypot(b.x - point.x, b.y - point.y)
        }), let pose = hit.pose {
          let marker = screen(x: pose.x, y: pose.y, size: geometry.size)
          if hypot(marker.x - point.x, marker.y - point.y) <= 22 {
            select(hit.id)
            return
          }
        }
        let world = LocalMapProjection.world(
          point, size: geometry.size, extent: extent, centerX: centerX, centerY: centerY)
        pick(world.0, world.1)
      }
    }
    .onChange(of: occupancy?.grid.run_id) { _, _ in fit() }
    .onChange(of: selected) { _, _ in fit() }
    .onAppear { fit() }

  }
  private func legend(_ name: String, _ color: Color) -> some View {
    HStack(spacing: 3) {
      Rectangle().fill(color).frame(width: 9, height: 9).overlay(
        Rectangle().stroke(.secondary, lineWidth: 0.5))
      Text(name).font(.caption2)
    }
  }
  private func fit() {
    if let g = occupancy?.grid {
      extent = max(1, Double(max(g.width, g.height)) * g.resolution * 0.55)
    }
  }
  private func screen(x: Double, y: Double, size: CGSize) -> CGPoint {
    LocalMapProjection.screen(
      x: x, y: y, size: size, extent: extent, centerX: centerX, centerY: centerY)
  }
}
