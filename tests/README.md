# Tests

Rust tests live here, grouped by crate:

- `<crate>/*.rs`: integration targets registered with `[[test]]` in that crate's manifest.
- `<crate>/unit/*.rs`: private unit modules included through `#[cfg(test)]` and
  `#[path]` from their original modules. This preserves access to private internals.
- `deployment/`: isolated GCP installer and certificate renewal checks.

Run from the repository root:

```sh
cargo test --workspace --locked
python3 -m unittest discover -s tests/deployment -v
```

GPU tests remain ignored unless explicitly requested on a graphics-capable host.
Zenoh tests need permission to bind loopback sockets. Production map/demo fixture
assets remain in `crates/terra-map/src/maps/fixtures` because the app also uses them.

The historical Unity C# tests already live in its engine-required
`spikes/unity-dashboard/Assets/Tests` folder. They are run by Unity's EditMode runner;
the Unity bridge's Rust tests are in `tests/argos-unity-bridge`.
