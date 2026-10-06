# Native dashboard verification

Captured 2026-10-06 against the local Bevy simulator with the bundled GMU tile
anchor. `macos-dashboard.png` shows live pose/status and an active accepted
goal. `ios-dashboard.png` is the retained screenshot from the passing iOS
XCUITest after waypoint arrival and cancellation.

The generated Swift smoke independently observed movement from the origin to
38.82981, -77.3075, reached `arrived` at 0.709 m remaining, and observed `idle`
after cancellation. The iOS UI test then drove to 38.82986, -77.3075, reached
arrival, and cancelled. These are simulator results, not physical-rover or
physical-phone verification.

macOS and iOS unit suites passed (3 tests each). The Rust suite passed 13 tests. Native Zenoh loopback, Rust
contract/state tests, FFI lifecycle, and Clippy also passed. macOS XCUITest could not initialize host automation mode;
its UI automation result is not counted as passing.
