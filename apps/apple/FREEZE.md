# Apple dashboard: legacy fallback

As of October 17, 2026, new ARGOS UI development targets `apps/web` and `crates/argos-server`. The SwiftUI operator station remains available as a legacy fallback for capabilities and trials not yet validated in the browser. TerraPhone's iOS vehicle runtime is separate and remains supported.

The earlier proposed Unity C3 transition and fixed October retirement schedule are superseded. No fallback tag or completed hardware trial is implied by this document. Preserve historical evidence; record the actual code revision and interface used in each trial.

Build the fallback from the checked-out revision with `scripts/build-apple.sh`, then use `apps/apple/ARGOS.xcodeproj`. Shared Rust changes must remain compatible with the FFI and native build. Necessary compatibility and safety fixes are allowed; new dashboard feature work belongs in the browser.

See the [browser guide](../../docs/browser-dashboard.md) and [legacy mission controls](../../docs/autonomy/README.md).
