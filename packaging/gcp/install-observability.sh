#!/bin/bash
# Optional observability release; run only after local acceptance and deployment review.
set -euo pipefail
cd /tmp/argos-release
[[ -f bin/terra-observability-collector && -f observability/credentials.json && -f replay/uv.lock ]] || {
    echo 'Missing collector, scoped credential hashes, or pinned replay source bundle' >&2
    exit 1
}
command -v uv >/dev/null || { echo 'Install uv before the reviewed observability release' >&2; exit 1; }
getent group terra-observability >/dev/null || groupadd --system terra-observability
id -u terra-observability >/dev/null 2>&1 || useradd --system --gid terra-observability --home-dir /var/lib/terra-observability --shell /usr/sbin/nologin terra-observability
install -d -o root -g root -m 755 /opt/argos/bin /opt/terra-observability
install -d -o root -g terra-observability -m 750 /etc/terra-observability
install -d -o terra-observability -g terra-observability -m 700 /var/lib/terra-observability
# Backups stay private; no changes to router credentials, database or TLS material.
if [[ -f /etc/terra-observability/credentials.json ]]; then
    install -o root -g root -m 600 /etc/terra-observability/credentials.json /etc/terra-observability/credentials.previous.json
fi
if [[ -f /opt/argos/bin/terra-observability-collector ]]; then
    install -o root -g root -m 755 /opt/argos/bin/terra-observability-collector /opt/argos/bin/terra-observability-collector.previous
fi
install -o root -g root -m 755 bin/terra-observability-collector /opt/argos/bin/terra-observability-collector
install -o root -g terra-observability -m 640 observability/credentials.json /etc/terra-observability/credentials.json
cp -R replay/. /opt/terra-observability/
uv sync --project /opt/terra-observability --frozen --no-dev
install -o root -g root -m 644 gcp/terra-observability.service /etc/systemd/system/
# Keep the previous proxy configuration recoverable even when validation fails.
rollback_dir=$(mktemp -d /etc/terra-observability/proxy-backup.XXXXXX)
chmod 700 "$rollback_dir"
for name in Caddyfile argos-observability.caddy; do
    if [[ -f /etc/caddy/$name ]]; then cp -p "/etc/caddy/$name" "$rollback_dir/$name"; fi
done
restore_proxy() {
    for name in Caddyfile argos-observability.caddy; do
        if [[ -f "$rollback_dir/$name" ]]; then
            cp -p "$rollback_dir/$name" "/etc/caddy/$name"
        else
            rm -f "/etc/caddy/$name"
        fi
    done
}
trap restore_proxy ERR
install -o root -g root -m 644 gcp/argos-observability.caddy /etc/caddy/argos-observability.caddy
# The router-only installer also installs this Caddyfile; its optional import survives upgrades.
install -o root -g root -m 644 gcp/Caddyfile /etc/caddy/Caddyfile
caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
systemctl daemon-reload
systemctl enable --now terra-observability
systemctl restart terra-observability
systemctl reload caddy
trap - ERR
printf 'Previous proxy configuration: %s\n' "$rollback_dir"
