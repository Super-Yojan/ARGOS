#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
swiftc apps/apple/Generated/ArgosCore.swift scripts/SwiftSmoke.swift \
 -module-cache-path target/swift-module-cache \
 -I apps/apple/Generated/headers -L target/debug -largos_ffi \
 -Xlinker -rpath -Xlinker "$PWD/target/debug" -o target/debug/argos-swift-smoke
target/debug/argos-swift-smoke "${1:-tcp/127.0.0.1:7447}"
