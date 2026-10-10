import Foundation
import SwiftUI

private actor RouterBackend {
  let service = ArgosRouter()
  func start(address: String) throws { try service.start(port: 7448, tailscaleAddress: address) }
  func stop() throws { try service.stop() }
  func status() -> (Bool, [String]) { (service.running(), service.logs()) }
}
@MainActor final class RouterController: ObservableObject {
  static let shared = RouterController()
  @Published private(set) var running = false
  @Published private(set) var busy = false
  @Published private(set) var logs: [String] = []
  @Published var error: String?
  private let backend = RouterBackend()
  private var advertisement: NetService?
  func start(address: String) async -> Bool {
    guard !busy else { return false }
    if running { return true }
    busy = true; error = nil
    defer { busy = false }
    do { try await backend.start(address: address) }
    catch { self.error = error.localizedDescription }
    (running, logs) = await backend.status()
    #if os(macOS)
    if running && address == "0.0.0.0" {
      let service = NetService(domain: "local.", type: "_argos-zenoh._tcp.", name: "ARGOS on \(Host.current().localizedName ?? "Mac")", port: 7448)
      advertisement = service; service.publish()
    }
    #endif
    return running
  }
  func stop() async {
    guard !busy else { return }; busy = true; error = nil
    defer { busy = false }
    advertisement?.stop(); advertisement = nil
    do { try await backend.stop() } catch { self.error = error.localizedDescription }
    (running, logs) = await backend.status()
  }
}

/// Bonjour is discovery only. Fleet traffic still uses the selected Zenoh TCP endpoint.
@MainActor final class RouterDiscovery: NSObject, ObservableObject, NetServiceBrowserDelegate, NetServiceDelegate {
  @Published private(set) var routers: [String] = []
  @Published private(set) var searching = false
  private let browser = NetServiceBrowser()
  private var services: [NetService] = []
  private var timeout: Task<Void, Never>?
  func find() {
    browser.stop(); services.removeAll(); routers.removeAll(); timeout?.cancel()
    searching = true; browser.delegate = self
    browser.searchForServices(ofType: "_argos-zenoh._tcp.", inDomain: "local.")
    timeout = Task { [weak self] in
      try? await Task.sleep(for: .seconds(8))
      guard !Task.isCancelled else { return }
      self?.browser.stop(); self?.searching = false
    }
  }
  func netServiceBrowser(_ browser: NetServiceBrowser, didFind service: NetService, moreComing: Bool) {
    services.append(service); service.delegate = self; service.resolve(withTimeout: 5)
  }
  func netServiceBrowser(_ browser: NetServiceBrowser, didNotSearch errorDict: [String: NSNumber]) { searching = false }
  func netServiceDidResolveAddress(_ sender: NetService) {
    guard let host = sender.hostName, sender.port > 0 else { return }
    let endpoint = "tcp/\(host):\(sender.port)"
    if !routers.contains(endpoint) { routers.append(endpoint); routers.sort() }
  }
}
