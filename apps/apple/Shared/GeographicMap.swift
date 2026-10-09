import SwiftUI
import MapKit
struct GeographicMap:View {
    let rovers:[RoverView]
    let selected:UInt64
    let latitude:Double
    let longitude:Double
    let draft:(Double,Double)?
    var select: (UInt64) -> Void = { _ in }
    let pick:(Double,Double)->Void
    @State private var camera:MapCameraPosition = .automatic
    var body:some View {
        MapReader {proxy in
            Map(position:$camera) {
                ForEach(rovers,id:\.id) {rover in
                    if let pose=rover.pose {
                        Annotation("Rover \(rover.id)",coordinate:MapProjection.coordinate(x:pose.x,y:pose.y,latitude:latitude,longitude:longitude)) {
                            Button { select(rover.id) } label: { Image(systemName:"location.north.circle.fill").font(.title).foregroundStyle((rover.poseAge ?? 99)<2.5 ? (rover.id==selected ? Color.blue:Color.teal):Color.gray).rotationEffect(.radians(-pose.yaw)).accessibilityLabel("Select Rover \(rover.id)") }.buttonStyle(.plain)
                        }
                    }
                    if let goal=rover.goal,goal.state != "idle" {Marker("Goal \(rover.id)",systemImage:"flag.fill",coordinate:MapProjection.coordinate(x:goal.x,y:goal.y,latitude:latitude,longitude:longitude)).tint(.orange)}
                }
                if let draft {Marker("Draft waypoint",systemImage:"scope",coordinate:CLLocationCoordinate2D(latitude:draft.0,longitude:draft.1)).tint(.purple)}
            }
            .mapStyle(.imagery(elevation:.realistic))
            .onTapGesture {location in if let coordinate=proxy.convert(location,from:.local) {pick(coordinate.latitude,coordinate.longitude)}}
            .onAppear {center()}
            .onChange(of:latitude) {_,_ in center()}
            .onChange(of:longitude) {_,_ in center()}
        }
    }
    private func center() {camera = .region(MKCoordinateRegion(center:CLLocationCoordinate2D(latitude:latitude,longitude:longitude),latitudinalMeters:180,longitudinalMeters:180))}
}
