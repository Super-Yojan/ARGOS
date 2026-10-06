import SwiftUI
struct ConnectionView: View {
    @EnvironmentObject var model:FleetModel
    @Environment(\.dismiss) private var dismiss
    @AppStorage("endpoint") private var endpoint="tcp/127.0.0.1:7447"
    @AppStorage("prefix") private var prefix="terra/rover"
    @AppStorage("geographic") private var geographic=false
    @AppStorage("anchorLatitude") private var anchorLatitude=38.8297
    @AppStorage("anchorLongitude") private var anchorLongitude = -77.3075
    var body:some View {
        NavigationStack {
            Form {
                Section("Terra connection") {
                    TextField("Endpoint",text:$endpoint).accessibilityIdentifier("endpoint")
                    TextField("Topic prefix",text:$prefix)
                    Text("On iPhone, use the simulator computer’s LAN address instead of 127.0.0.1.").font(.caption).foregroundStyle(.secondary)
                }
                Section("World") {
                    Toggle("Geographic map",isOn:$geographic)
                    if geographic {
                        TextField("Anchor latitude",value:$anchorLatitude,format:.number)
                        TextField("Anchor longitude",value:$anchorLongitude,format:.number)
                        Text("Match Terra’s tile anchor. Default: GMU Johnson Center.").font(.caption).foregroundStyle(.secondary)
                    } else {Text("Local coordinates in metres. +x is forward/north; +y is left/west.").font(.caption).foregroundStyle(.secondary)}
                }
                Section {
                    Button(model.busy ? "Connecting…":"Connect") {
                        guard anchorLatitude.isFinite,anchorLongitude.isFinite,abs(anchorLatitude)<=85,abs(anchorLongitude)<=180 else {model.error="Enter a valid Terra map anchor.";return}
                        Task {await model.connect(endpoint:endpoint,prefix:prefix);if model.snapshot.link != "disconnected" {dismiss()}}
                    }.disabled(model.busy).accessibilityIdentifier("connect")
                    if model.snapshot.link != "disconnected" {Button("Disconnect",role:.destructive) {Task{await model.disconnect();dismiss()}}}
                }
            }
            .formStyle(.grouped).navigationTitle("Connection")
            .toolbar {ToolbarItem(placement:.confirmationAction) {Button("Done") {dismiss()}}}
        }
        #if os(macOS)
        .frame(width:480,height:520)
        #endif
    }
}
