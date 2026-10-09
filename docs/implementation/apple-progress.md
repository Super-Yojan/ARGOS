# Apple dashboard execution ledger

!!! tip "TL;DR"
    Apple Silicon only.
    Simulator waypoint and cancel passed.
    A signed iPhone on a real LAN is still unverified.

![Mac dashboard from the simulator acceptance run.](../assets/macos-dashboard.jpg)

*The console the ledger is about.*

Plan: docs/superpowers/plans/2026-10-06-operator-dashboard.md
Base: 0852aac
Ruling: isolated ARGOS checkout at /private/tmp/argos-apple on codex/apple-dashboard; app-managed worktree tool targets the chat's Masters-Thesis repository, so manual ARGOS worktree used.
Ruling: implement pure core contracts before transport smoke to keep protocol behavior independently testable. Native Apple networking verification still gates release claims.
Preflight: core owned records -> transport snapshot -> UniFFI -> Swift; same field names throughout. Existing Python CLI remains independently runnable.
Tasks: contract/telemetry in progress; sessions, bindings, UI, simulator acceptance, compatibility pending.
Core tests: 10 passed; native Zenoh loopback: 1 passed (localhost requires escalation); FFI lifecycle test: 1 passed.
Apple packaging: XCFramework generated for Apple Silicon macOS, iOS device, and iOS simulator. Intel Mac not installed in toolchain; support deferred and explicitly documented.
Ruling: 5-Hz actor-owned Swift polling avoids FFI callbacks and keeps network work off MainActor. Swift coordinate conversion mirrors tested Rust constant for map interaction.
Apple tests: macOS 2/2 unit tests; iOS Simulator 2/2 unit tests. iOS live UI flow passed (waypoint arrived and cancel acknowledged) with screenshot retained in xcresult.
Real Bevy simulator Swift smoke: rover pose received, geographic waypoint reached arrived within 0.75m, cancel returned idle. Mac dashboard visibly shows fresh pose/status and accepted goal.
Mac XCUITest blocked by host automation-mode initialization timeout; no system permission was changed. Native Mac unit tests and direct Swift simulator verification passed.
Python compatibility: 37/37 tests passed. This is the only Python product change; native build/tooling uses Cargo, shell, Ruby, and Swift.
Physical iPhone signing/LAN run, active-goal cancel->teleop movement, and full restart/background end-to-end checks remain unverified. No claim of those checks passing.
User amendment: remove old Python artifacts. Removed legacy CLI/package/tests, pyproject.toml, Python CI, local generated environment/caches, and obsolete setup/architecture text. This supersedes the earlier compatibility requirement and its prior test results.
Final review: independent reviewer found two important issues, both corrected with red->green regressions: cap retired rover rows at 32 and preserve selected stale/absent rows while available. No additional important command, FFI, or lifecycle bugs found.
Final checks: Rust suite 13/13, Clippy -D warnings clean, macOS Swift unit tests 3/3, iOS Simulator Swift unit tests 3/3, regenerated three-slice XCFramework. Live iOS UI acceptance and Swift simulator smoke passed before the selection/storage-only fixes; Mac automation remains blocked as documented.
Ruling: initial Apple Silicon support only; cost is no Intel Mac binary until a universal slice is added. World/anchor must be configured to match Terra because the current bus does not advertise them.
