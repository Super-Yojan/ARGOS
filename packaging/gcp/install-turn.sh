#!/bin/bash
# FPV relay (TURN) on the router VM. Run as root on argos-host:
#   sudo bash install-turn.sh
#
# WebRTC video goes phone -> Mac directly. When the two cannot reach each
# other (campus Wi-Fi, cellular, iOS Local Network permission off, strict
# NAT) the call fails after ~15 s. coturn relays the video through this VM
# instead. Safe to rerun: the password in /etc/argos/turn.password is kept.
set -euo pipefail

host=argos.34.135.204.210.sslip.io
public_ip=34.135.204.210
user=argos-fpv
min_port=49160
max_port=49200

export DEBIAN_FRONTEND=noninteractive
command -v turnserver >/dev/null || { apt-get update -q; apt-get install -y -q coturn; }

install -d -o root -g argos -m 750 /etc/argos
secret=/etc/argos/turn.password
if [ ! -s "$secret" ]; then
    python3 -c 'import secrets; print(secrets.token_urlsafe(24))' >"$secret"
fi
chmod 600 "$secret"
password=$(tr -d '\n' <"$secret")

# GCE: the VM only sees its internal address; the public one is 1:1 NAT.
private_ip=$(curl -fsS -H 'Metadata-Flavor: Google' \
    http://metadata.google.internal/computeMetadata/v1/instance/network-interfaces/0/ip)

# coturn reads the router's TLS copy (refreshed by argos-router-tls).
id -u turnserver >/dev/null 2>&1 && usermod -a -G argos turnserver

umask 077
cat >/etc/turnserver.conf <<EOF
# Managed by ARGOS packaging/gcp/install-turn.sh. Rerun it instead of editing.
listening-port=3478
tls-listening-port=5349
listening-ip=$private_ip
relay-ip=$private_ip
external-ip=$public_ip/$private_ip
min-port=$min_port
max-port=$max_port
realm=$host
server-name=$host
fingerprint
lt-cred-mech
user=$user:$password
cert=/var/lib/argos/tls/server.crt
pkey=/var/lib/argos/tls/server.key
no-tlsv1
no-tlsv1_1
no-cli
no-multicast-peers
no-software-attribute
# Relay only to the public internet, never into the VM's own network.
denied-peer-ip=0.0.0.0-0.255.255.255
denied-peer-ip=10.0.0.0-10.255.255.255
denied-peer-ip=100.64.0.0-100.127.255.255
denied-peer-ip=127.0.0.0-127.255.255.255
denied-peer-ip=169.254.0.0-169.254.255.255
denied-peer-ip=172.16.0.0-172.31.255.255
denied-peer-ip=192.168.0.0-192.168.255.255
denied-peer-ip=::1
denied-peer-ip=fc00::-fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff
denied-peer-ip=fe80::-febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff
# A few rovers at ~2 Mbit/s each.
user-quota=12
total-quota=60
log-file=syslog
simple-log
EOF
chown root:turnserver /etc/turnserver.conf 2>/dev/null || true
chmod 640 /etc/turnserver.conf

if [ -f /etc/default/coturn ]; then
    sed -i 's/^#\?TURNSERVER_ENABLED=.*/TURNSERVER_ENABLED=1/' /etc/default/coturn
fi
systemctl enable coturn
systemctl restart coturn
sleep 1
systemctl is-active --quiet coturn

cat <<EOF
coturn is running.
  ARGOS .env:   ARGOS_FPV_TURN_URL=turn:$host:3478
                ARGOS_FPV_TURN_USERNAME=$user
                ARGOS_FPV_TURN_CREDENTIAL=<sudo cat $secret>
  TerraPhone:   FPV settings -> TURN URL turn:$host:3478, same user and password.
Open the firewall once (from a machine with gcloud): see packaging/gcp/README.md.
EOF
