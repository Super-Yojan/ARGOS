#!/bin/bash
# Run as root after extracting the release bundle to /tmp/argos-release.
set -euo pipefail
cd /tmp/argos-release
install -o root -g root -m 755 bin/* /opt/argos/bin/
mkdir -p /opt/argos/web
if [ -d web ]; then
  cp -a web/. /opt/argos/web/
fi
install -o root -g root -m 644 gcp/*.service gcp/*.timer /etc/systemd/system/
install -o root -g root -m 755 gcp/refresh-router-tls.sh /opt/argos/bin/
install -o root -g root -m 644 gcp/Caddyfile /etc/caddy/Caddyfile
python3 - <<'PY'
import pathlib,secrets,json
root=pathlib.Path('/etc/argos')
secret=root/'fleet.password'
if not secret.exists():
 secret.write_text(secrets.token_urlsafe(32)+'\n');secret.chmod(0o600)
password=secret.read_text().strip()
(root/'users.txt').write_text('terra:'+password+'\nargos:'+password+'\n')
config={"mode":"router","listen":{"endpoints":["tls/0.0.0.0:7448","tcp/127.0.0.1:7449"]},"scouting":{"multicast":{"enabled":False}},"transport":{"auth":{"usrpwd":{"dictionary_file":"/etc/argos/users.txt"}},"link":{"tls":{"listen_private_key":"/var/lib/argos/tls/server.key","listen_certificate":"/var/lib/argos/tls/server.crt","enable_mtls":False}}}}
(root/'router.json5').write_text(json.dumps(config))
env=root/'server.env'
if not env.exists():
 env.write_text('ARGOS_ENV=production\nARGOS_BIND=127.0.0.1:8080\nARGOS_ORIGIN=https://argos.34.135.204.210.sslip.io\nARGOS_FLEET=zenoh\nARGOS_ZENOH_ENDPOINT=tcp/127.0.0.1:7449\nARGOS_ZENOH_PREFIX=terra/phone\nARGOS_ZENOH_USER=argos\nARGOS_ZENOH_PASSWORD='+password+'\nARGOS_WEB_DIR=/opt/argos/web\nARGOS_DATABASE=sqlite:///var/lib/argos/argos.db\nARGOS_OPERATORS=33645913\nGITHUB_CLIENT_ID=\nGITHUB_CLIENT_SECRET=\n')
 env.chmod(0o600)
PY
chown root:argos /etc/argos/users.txt /etc/argos/router.json5
chmod 640 /etc/argos/users.txt /etc/argos/router.json5
systemctl daemon-reload
systemctl enable --now caddy
systemctl enable argos-router argos-server
systemctl enable --now argos-router-tls.timer
# OAuth credentials must be installed before starting production server.
if grep -q '^GITHUB_CLIENT_SECRET=.' /etc/argos/server.env; then
    systemctl restart argos-server
fi
