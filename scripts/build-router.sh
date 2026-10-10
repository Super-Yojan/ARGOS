#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo build --locked --release -p argos-zenoh --bin argos-router
printf 'Router binary: %s/target/release/argos-router\n' "$PWD"
