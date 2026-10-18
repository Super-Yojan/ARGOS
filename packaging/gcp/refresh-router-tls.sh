#!/bin/bash
set -euo pipefail
host=argos.34.135.204.210.sslip.io
base=/var/lib/caddy/.local/share/caddy/certificates
cert=$(find "$base" -type f -path "*/$host/$host.crt" -print -quit)
key=${cert%.crt}.key
[ -s "$cert" ] && [ -s "$key" ]
install -d -o root -g argos -m 750 /var/lib/argos/tls
if ! cmp -s "$cert" /var/lib/argos/tls/server.crt || ! cmp -s "$key" /var/lib/argos/tls/server.key; then
    install -o root -g argos -m 640 "$cert" /var/lib/argos/tls/server.crt
    install -o root -g argos -m 640 "$key" /var/lib/argos/tls/server.key
    systemctl restart argos-router.service
fi
systemctl start argos-router.service
