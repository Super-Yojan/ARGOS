#!/bin/bash
# Build an independent observer bundle; never contact or modify the GCP host.
set -euo pipefail
argos_root=$(cd "$(dirname "$0")/../.." && pwd)
terra_source=${TERRA_SOURCE:-"$(dirname "$argos_root")/Terra"}
[[ -f "$terra_source/crates/terra-observability-collector/Cargo.toml" ]] || {
    echo 'Place the reviewed Terra checkout beside ARGOS, or set TERRA_SOURCE' >&2
    exit 1
}
command -v cargo-zigbuild >/dev/null && command -v zig >/dev/null || {
    echo 'Install cargo-zigbuild, Zig and the x86_64-unknown-linux-gnu Rust target first' >&2
    exit 1
}
cd "$terra_source"
cargo zigbuild --release --locked --target x86_64-unknown-linux-gnu.2.36 -p terra-observability-collector
release_dir=$(mktemp -d /tmp/argos-observability-release.XXXXXX)
mkdir -p "$release_dir"/{bin,gcp,replay,observability,docs}
cp target/x86_64-unknown-linux-gnu/release/terra-observability-collector "$release_dir/bin/"
for name in Caddyfile terra-observability.service terra-rerun-viewer.service argos-observability.caddy install-observability.sh; do
    cp "$argos_root/packaging/gcp/$name" "$release_dir/gcp/"
done
cp tools/observability/{pyproject.toml,uv.lock,README.md} "$release_dir/replay/"
cp -R tools/observability/terra_replay "$release_dir/replay/"
find "$release_dir/replay" -type d -name __pycache__ -exec rm -rf {} +
cp docs/observability/{operations.md,source-settings.example.json,event-schema.md,coverage.md} "$release_dir/docs/"
cat > "$release_dir/observability/README.txt" <<'EOF'
This bundle contains no credentials and cannot start without provisioned scopes.
Privately place the provisioner's HASH-ONLY credentials.json in this directory.
Raw writer/reader tokens belong only on their scoped clients, never in this bundle.
Review the Caddy hostname before installation. See docs/operations.md.
On the host, unpack at /tmp/argos-release and run the reviewed installer explicitly.
EOF
git rev-parse HEAD > "$release_dir/docs/terra-source-revision.txt"
git -C "$argos_root" rev-parse HEAD > "$release_dir/docs/argos-source-revision.txt"
python3 - "$release_dir" <<'PY'
from pathlib import Path
import hashlib, sys
root = Path(sys.argv[1])
lines = []
for path in sorted(p for p in root.rglob('*') if p.is_file()):
    lines.append(hashlib.sha256(path.read_bytes()).hexdigest() + '  ' + str(path.relative_to(root)))
(root / 'SHA256SUMS').write_text('\n'.join(lines) + '\n')
PY
tar -czf "$release_dir.tar.gz" -C "$release_dir" .
printf 'Prepared observer bundle: %s.tar.gz\n' "$release_dir"
