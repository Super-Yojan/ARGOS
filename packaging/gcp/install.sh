#!/bin/bash
# Run as root after extracting the router release to /tmp/argos-release.
set -euo pipefail
cd /tmp/argos-release
getent group argos >/dev/null || groupadd --system argos
id -u argos >/dev/null 2>&1 || useradd --system --gid argos --home-dir /var/lib/argos --shell /usr/sbin/nologin argos
install -d -o root -g root -m 755 /opt/argos/bin
install -d -o root -g argos -m 750 /etc/argos /var/lib/argos
install -o root -g root -m 755 bin/argos-router /opt/argos/bin/argos-router
install -o root -g root -m 644 gcp/argos-router.service gcp/argos-router-tls.service gcp/argos-router-tls.timer /etc/systemd/system/
install -o root -g root -m 755 gcp/refresh-router-tls.sh /opt/argos/bin/
install -o root -g root -m 644 gcp/Caddyfile /etc/caddy/Caddyfile
python3 - <<'PY'
import pathlib, secrets, json
root = pathlib.Path('/etc/argos')
secret = root / 'fleet.password'
if not secret.exists():
    secret.write_text(secrets.token_urlsafe(32) + '\n')
secret.chmod(0o600)
password = secret.read_text().strip()
if not password:
    raise SystemExit('Existing fleet.password is empty; restore the fleet credential before installing')
users = root / 'users.txt'
# Preserve custom accounts and credentials on upgrades.
if not users.exists():
    users.write_text('terra:' + password + '\nargos:' + password + '\n')
# Zorvane players sign in as `zorvane` and may only touch zorvane/**.
zorvane = root / 'zorvane.password'
if not zorvane.exists():
    zorvane.write_text(secrets.token_urlsafe(18) + '\n')
zorvane.chmod(0o600)
lines = users.read_text().splitlines()
names = [line.split(':', 1)[0] for line in lines if ':' in line]
if 'zorvane' not in names:
    lines.append('zorvane:' + zorvane.read_text().strip())
    users.write_text('\n'.join(lines) + '\n')
fleet = sorted({name for name in names if name != 'zorvane'})
messages = ['put', 'delete', 'declare_subscriber', 'query', 'declare_queryable', 'reply',
            'liveliness_token', 'declare_liveliness_subscriber', 'liveliness_query']
config = {
    'mode': 'router',
    'listen': {'endpoints': ['tls/0.0.0.0:7448', 'tcp/127.0.0.1:7449']},
    'scouting': {'multicast': {'enabled': False}},
    'transport': {
        'auth': {'usrpwd': {'dictionary_file': '/etc/argos/users.txt'}},
        'link': {'tls': {
            'listen_private_key': '/var/lib/argos/tls/server.key',
            'listen_certificate': '/var/lib/argos/tls/server.crt',
            'enable_mtls': False,
        }},
    },
    # Default deny: every account in users.txt except zorvane keeps full access.
    # Rerun install.sh after adding an account so it joins the fleet subject.
    'access_control': {
        'enabled': True,
        'default_permission': 'deny',
        'rules': [
            {'id': 'everything', 'key_exprs': ['**'], 'messages': messages,
             'flows': ['ingress', 'egress'], 'permission': 'allow'},
            {'id': 'zorvane-only', 'key_exprs': ['zorvane/**'], 'messages': messages,
             'flows': ['ingress', 'egress'], 'permission': 'allow'},
        ],
        'subjects': [
            {'id': 'fleet', 'usernames': fleet},
            {'id': 'players', 'usernames': ['zorvane']},
        ],
        'policies': [
            {'rules': ['everything'], 'subjects': ['fleet']},
            {'rules': ['zorvane-only'], 'subjects': ['players']},
        ],
    },
}
(root / 'router.json5').write_text(json.dumps(config))
PY
chown root:argos /etc/argos/users.txt /etc/argos/router.json5
chmod 640 /etc/argos/users.txt /etc/argos/router.json5
# Retire the HTTP service on existing hosts; retain its data for recovery.
if systemctl list-unit-files argos-server.service --no-legend | grep -q '^argos-server.service'; then
    systemctl disable --now argos-server.service
fi
rm -f /etc/systemd/system/argos-server.service
systemctl daemon-reload
caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
systemctl enable --now caddy
systemctl reload caddy
systemctl enable argos-router
systemctl enable --now argos-router-tls.timer
# Caddy may still be obtaining the first certificate. The timer retries.
if ! systemctl start argos-router-tls.service; then
    echo 'TLS refresh is pending; inspect argos-router-tls.service. The timer will retry.' >&2
elif systemctl is-active --quiet argos-router; then
    # Load a newly installed binary/config even when the certificate is unchanged.
    systemctl restart argos-router
fi
