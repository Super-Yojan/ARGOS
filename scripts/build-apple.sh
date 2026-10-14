#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
generated="$root/apps/apple/Generated"
# Xcode's configuration does not optimize this prebuilt Rust library. Ship
# optimized slices by default; opt into debug explicitly for Rust debugging.
rust_profile=${ARGOS_RUST_PROFILE:-release}
case "$rust_profile" in
 release) cargo_profile=release ;;
 debug) cargo_profile=dev ;;
 *) echo "ARGOS_RUST_PROFILE must be release or debug" >&2; exit 1 ;;
esac
mkdir -p "$generated/headers"
cargo build -p argos-ffi --profile "$cargo_profile"
cargo run -p argos-ffi --features bindgen --bin argos-bindgen -- generate \
 --library "target/$rust_profile/libargos_ffi.dylib" --language swift --config crates/argos-ffi/uniffi.toml --out-dir "$generated"
cp "$generated/ArgosCoreFFI.h" "$generated/headers/"
cat > "$generated/headers/module.modulemap" <<'MODULE'
module ArgosCoreFFI { header "ArgosCoreFFI.h" export * }
MODULE
# Apple Silicon Mac, iOS device, and Apple Silicon simulator are distinct slices.
for platform in aarch64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim; do
 cargo build -p argos-ffi --target "$platform" --profile "$cargo_profile"
done
# Delete only this script's generated bundle.
rm -rf "$generated/ArgosCore.xcframework"
xcodebuild -create-xcframework \
 -library "target/aarch64-apple-darwin/$rust_profile/libargos_ffi.a" -headers "$generated/headers" \
 -library "target/aarch64-apple-ios/$rust_profile/libargos_ffi.a" -headers "$generated/headers" \
 -library "target/aarch64-apple-ios-sim/$rust_profile/libargos_ffi.a" -headers "$generated/headers" \
 -output "$generated/ArgosCore.xcframework"
