#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
generated="$root/apps/apple/Generated"
mkdir -p "$generated/headers"
cargo build -p argos-ffi
cargo run -p argos-ffi --features bindgen --bin argos-bindgen -- generate \
 --library target/debug/libargos_ffi.dylib --language swift --config crates/argos-ffi/uniffi.toml --out-dir "$generated"
cp "$generated/ArgosCoreFFI.h" "$generated/headers/"
cat > "$generated/headers/module.modulemap" <<'MODULE'
module ArgosCoreFFI { header "ArgosCoreFFI.h" export * }
MODULE
# Apple Silicon Mac, iOS device, and Apple Silicon simulator are distinct slices.
for platform in aarch64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim; do
 cargo build -p argos-ffi --target "$platform"
done
# Delete only this script's generated bundle.
rm -rf "$generated/ArgosCore.xcframework"
xcodebuild -create-xcframework \
 -library target/aarch64-apple-darwin/debug/libargos_ffi.a -headers "$generated/headers" \
 -library target/aarch64-apple-ios/debug/libargos_ffi.a -headers "$generated/headers" \
 -library target/aarch64-apple-ios-sim/debug/libargos_ffi.a -headers "$generated/headers" \
 -output "$generated/ArgosCore.xcframework"
