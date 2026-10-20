#!/bin/sh
set -eu
argos_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$argos_root"
exec cargo run -p argos-gpui -- --sim "$@"
