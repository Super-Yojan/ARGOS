"""Exercise host mutations in a temporary filesystem; never contact GCP."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class DeploymentTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bin = self.root / 'mock-bin'
        self.bin.mkdir()
        self.events = self.root / 'events'
        self.env = dict(os.environ, PATH=f'{self.bin}:{os.environ["PATH"]}', EVENTS=str(self.events))
        self.mock('systemctl', '''#!/bin/sh
printf '%s\n' "$*" >> "$EVENTS"
if [ "$1" = list-unit-files ]; then echo 'argos-server.service enabled'; fi
exit 0
''')
        self.mock('caddy', '#!/bin/sh\nprintf "caddy %s\\n" "$*" >> "$EVENTS"\n')
        for cmd in ('getent', 'id', 'chown'):
            self.mock(cmd, '#!/bin/sh\nexit 0\n')
        self.mock('install', '''#!/usr/bin/env python3
import pathlib, shutil, sys
args=sys.argv[1:]; paths=[]; directory=False
while args:
    a=args.pop(0)
    if a in ('-o','-g','-m'): args.pop(0)
    elif a=='-d': directory=True
    else: paths.append(pathlib.Path(a))
if directory:
    for p in paths: p.mkdir(parents=True,exist_ok=True)
else:
    dest=paths[-1]
    for source in paths[:-1]:
        target=dest/source.name if dest.is_dir() else dest
        target.parent.mkdir(parents=True,exist_ok=True)
        shutil.copyfile(source,target)
''')
        for rel in ('etc/systemd/system', 'etc/caddy', 'etc/argos', 'var/lib/argos', 'release/bin'):
            (self.root / rel).mkdir(parents=True)
        (self.root / 'release/bin/argos-router').write_text('router')
        shutil.copytree(ROOT / 'packaging/gcp', self.root / 'release/gcp')

    def mock(self, name, script):
        p = self.bin / name
        p.write_text(script)
        p.chmod(0o755)

    def run_script(self, name):
        source = (ROOT / 'packaging/gcp' / name).read_text()
        for prefix, replacement in (
            ('/tmp/argos-release', str(self.root / 'release')),
            ('/opt/argos', str(self.root / 'opt/argos')),
            ('/etc/argos', str(self.root / 'etc/argos')),
            ('/etc/systemd/system', str(self.root / 'etc/systemd/system')),
            ('/etc/caddy', str(self.root / 'etc/caddy')),
            ('/var/lib/argos', str(self.root / 'var/lib/argos')),
            ('/var/lib/caddy', str(self.root / 'var/lib/caddy')),
        ):
            source = source.replace(prefix, replacement)
        subprocess.run(['bash', '-c', source], env=self.env, check=True, capture_output=True)

    def test_fresh_install_keeps_router_auth_and_tls(self):
        self.run_script('install.sh')
        config = json.loads((self.root / 'etc/argos/router.json5').read_text())
        self.assertEqual(config['listen']['endpoints'], ['tls/0.0.0.0:7448', 'tcp/127.0.0.1:7449'])
        transport = config['transport']
        self.assertTrue(transport['auth']['usrpwd']['dictionary_file'].endswith('/users.txt'))
        self.assertTrue(transport['link']['tls']['listen_private_key'].endswith('/tls/server.key'))
        secret = self.root / 'etc/argos/fleet.password'
        self.assertEqual(secret.stat().st_mode & 0o777, 0o600)
        self.assertIn('terra:' + secret.read_text().strip(), (self.root / 'etc/argos/users.txt').read_text())
        self.assertFalse((self.root / 'etc/argos/server.env').exists())
        zorvane = self.root / 'etc/argos/zorvane.password'
        self.assertEqual(zorvane.stat().st_mode & 0o777, 0o600)
        self.assertIn('zorvane:' + zorvane.read_text().strip(), (self.root / 'etc/argos/users.txt').read_text())

    def test_router_acl_limits_zorvane_players_to_their_keys(self):
        self.run_script('install.sh')
        acl = json.loads((self.root / 'etc/argos/router.json5').read_text())['access_control']
        self.assertTrue(acl['enabled'])
        self.assertEqual(acl['default_permission'], 'deny')
        rules = {rule['id']: rule for rule in acl['rules']}
        subjects = {subject['id']: subject['usernames'] for subject in acl['subjects']}
        self.assertEqual(subjects, {'fleet': ['argos', 'terra'], 'players': ['zorvane']})
        self.assertEqual(rules['zorvane-only']['key_exprs'], ['zorvane/**'])
        self.assertIn({'rules': ['zorvane-only'], 'subjects': ['players']}, acl['policies'])
        self.assertIn({'rules': ['everything'], 'subjects': ['fleet']}, acl['policies'])
        events = self.events.read_text()
        self.assertIn('enable --now argos-router-tls.timer', events)
        self.assertIn('reload caddy', events)
        self.assertIn('restart argos-router', events)
        self.assertNotIn('enable argos-router argos-server', events)

    def test_upgrade_preserves_credentials_and_server_data(self):
        for name, content in [('fleet.password', 'existing-secret\n'), ('users.txt', 'terra:existing-secret\ncustom:other\n'), ('server.env', 'legacy-data')]:
            (self.root / 'etc/argos' / name).write_text(content)
        unit = self.root / 'etc/systemd/system/argos-server.service'
        unit.write_text('old service')
        self.run_script('install.sh')
        self.assertEqual((self.root / 'etc/argos/fleet.password').read_text(), 'existing-secret\n')
        zorvane = (self.root / 'etc/argos/zorvane.password').read_text().strip()
        self.assertEqual((self.root / 'etc/argos/users.txt').read_text(),
                         f'terra:existing-secret\ncustom:other\nzorvane:{zorvane}\n')
        acl = json.loads((self.root / 'etc/argos/router.json5').read_text())['access_control']
        self.assertEqual(acl['subjects'][0], {'id': 'fleet', 'usernames': ['custom', 'terra']})
        # A second install keeps the zorvane account as it is.
        self.run_script('install.sh')
        self.assertEqual((self.root / 'etc/argos/users.txt').read_text().count('zorvane:'), 1)
        self.assertEqual((self.root / 'etc/argos/server.env').read_text(), 'legacy-data')
        self.assertFalse(unit.exists())
        self.assertIn('disable --now argos-server.service', self.events.read_text())

    def test_certificate_rotation_copies_material_and_restarts_only_on_change(self):
        host = 'argos.34.135.204.210.sslip.io'
        source = self.root / 'var/lib/caddy/.local/share/caddy/certificates/issuer' / host
        source.mkdir(parents=True)
        cert = source / f'{host}.crt'
        key = source / f'{host}.key'
        cert.write_text('first-cert')
        key.write_text('first-key')
        self.run_script('refresh-router-tls.sh')
        self.assertEqual((self.root / 'var/lib/argos/tls/server.crt').read_text(), 'first-cert')
        self.assertEqual((self.root / 'var/lib/argos/tls/server.key').read_text(), 'first-key')
        self.assertIn('restart argos-router.service', self.events.read_text())
        self.events.write_text('')
        self.run_script('refresh-router-tls.sh')
        self.assertNotIn('restart', self.events.read_text())
        cert.write_text('renewed-cert')
        self.run_script('refresh-router-tls.sh')
        self.assertEqual((self.root / 'var/lib/argos/tls/server.crt').read_text(), 'renewed-cert')
        self.assertIn('restart argos-router.service', self.events.read_text())

    def test_caddy_retains_certificate_hostname_without_http_upstream(self):
        config = (ROOT / 'packaging/gcp/Caddyfile').read_text()
        self.assertIn('argos.34.135.204.210.sslip.io {', config)
        self.assertNotIn('reverse_proxy', config)
        self.assertNotIn('auto_https off', config)
        self.assertIn('respond /healthz', config)


if __name__ == '__main__':
    unittest.main()
