import Foundation
@main enum SwiftSmoke { static func main() throws {
let client=ArgosClient()
try client.connect(config:ConnectionConfig(endpoint:CommandLine.arguments.count>1 ? CommandLine.arguments[1]:"tcp/127.0.0.1:7447",prefix:"terra/rover"))
defer {client.disconnect()}
let deadline=Date().addingTimeInterval(10)
while client.snapshot().rovers.isEmpty && Date()<deadline {Thread.sleep(forTimeInterval:0.1)}
let initial=client.snapshot()
guard let rover=initial.rovers.first else {fatalError("No live fleet state")}
let poseDeadline=Date().addingTimeInterval(10)
while client.snapshot().rovers.first?.pose == nil && Date()<poseDeadline {Thread.sleep(forTimeInterval:0.1)}
guard let pose=client.snapshot().rovers.first?.pose else {fatalError("No depth body pose")}
print("LIVE rover=\(rover.id) x=\(pose.x) y=\(pose.y)")
let token=try client.sendGoal(roverId:rover.id,waypoint:.geographic(latitude:38.82981,longitude:-77.3075,yaw:nil))
let arrivalDeadline=Date().addingTimeInterval(45)
var arrived=false
while Date()<arrivalDeadline {
    let snapshot=client.snapshot()
    if let row=snapshot.rovers.first,let goal=row.goal {
        print("GOAL \(goal.state) distance=\(goal.distance) token=\(goal.token ?? "none") phase=\(row.commandPhase)")
        if goal.token==token && goal.state=="arrived" {arrived=true;break}
    }
    Thread.sleep(forTimeInterval:0.5)
}
guard arrived else {fatalError("Simulated rover did not arrive")}
try client.cancelGoal(roverId:rover.id)
let cancelDeadline=Date().addingTimeInterval(5)
while client.snapshot().rovers.first?.commandPhase != "cancelled" && Date()<cancelDeadline {Thread.sleep(forTimeInterval:0.1)}
guard client.snapshot().rovers.first?.goal?.state=="idle" else {fatalError("Cancel did not release goal")}
print("ARRIVED and CANCELLED through generated Swift bindings")

}}
