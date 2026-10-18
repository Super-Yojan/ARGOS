#!/bin/bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends ca-certificates caddy curl sqlite3
systemctl stop caddy
id argos >/dev/null 2>&1 || useradd --system --home-dir /var/lib/argos --create-home --shell /usr/sbin/nologin argos
install -d -o argos -g argos -m 750 /var/lib/argos
install -d -o root -g argos -m 750 /etc/argos /opt/argos/bin
install -d -o root -g root -m 755 /opt/argos/web
