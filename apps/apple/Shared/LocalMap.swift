import SwiftUI
struct LocalMap:View {
    let rovers:[RoverView]
    let selected:UInt64
    let draft:(Double,Double)?
    let pick:(Double,Double)->Void
    @State private var extent=30.0
    var body:some View {
        GeometryReader {geometry in
            ZStack(alignment:.topLeading) {
                Canvas {context,size in
                    let middle=CGPoint(x:size.width/2,y:size.height/2)
                    var grid=Path()
                    for i in -3...3 {let offset=Double(i)/3;grid.move(to:CGPoint(x:middle.x+offset*size.width/2,y:0));grid.addLine(to:CGPoint(x:middle.x+offset*size.width/2,y:size.height));grid.move(to:CGPoint(x:0,y:middle.y+offset*size.height/2));grid.addLine(to:CGPoint(x:size.width,y:middle.y+offset*size.height/2))}
                    context.stroke(grid,with:.color(.secondary.opacity(0.2)),lineWidth:1)
                    for rover in rovers {
                        if let pose=rover.pose {
                            let point=screen(x:pose.x,y:pose.y,size:size)
                            context.fill(Path(ellipseIn:CGRect(x:point.x-6,y:point.y-6,width:12,height:12)),with:.color((rover.poseAge ?? 99)<2.5 ? (rover.id==selected ? .blue:.teal):.gray))
                            context.draw(Text("\(rover.id)").font(.caption),at:CGPoint(x:point.x,y:point.y+17))
                        }
                        if let goal=rover.goal,goal.state != "idle" {context.draw(Text("⚑").foregroundStyle(.orange),at:screen(x:goal.x,y:goal.y,size:size))}
                    }
                    if let draft {context.draw(Text("⊕").font(.title).foregroundStyle(.purple),at:screen(x:draft.0,y:draft.1,size:size))}
                }.background(Color.secondary.opacity(0.05))
                VStack(alignment:.leading,spacing:4) {Text("LOCAL WORLD").font(.caption.weight(.semibold));Text("↑ +x north · ← +y west").font(.caption);Text("±\(Int(extent)) metres").font(.caption)}.padding(12).allowsHitTesting(false)
                VStack {Spacer();HStack {Spacer();Button {extent=min(10000,extent*2)}label:{Image(systemName:"minus.magnifyingglass")};Button {extent=max(5,extent/2)}label:{Image(systemName:"plus.magnifyingglass")}}}.padding(12)
            }
            .onTapGesture {point in pick((geometry.size.height/2-point.y)/(geometry.size.height/2)*extent,(geometry.size.width/2-point.x)/(geometry.size.width/2)*extent)}
        }.accessibilityLabel("Local rover map. Set exact coordinates using the waypoint fields.")
    }
    private func screen(x:Double,y:Double,size:CGSize)->CGPoint {CGPoint(x:size.width/2-y/extent*size.width/2,y:size.height/2-x/extent*size.height/2)}
}
