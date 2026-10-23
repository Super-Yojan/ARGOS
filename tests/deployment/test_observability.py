"""Validate observability release installation in a temporary host, without GCP."""
import hashlib,json,os,pathlib,subprocess,tempfile,unittest,shutil
from pathlib import Path
ROOT=pathlib.Path(__file__).resolve().parents[2]

class ObservationHost(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bin = self.root / 'mock-bin'
        self.bin.mkdir()
        self.events = self.root / 'events'
        self.env = dict(os.environ, PATH=f'{self.bin}:{os.environ["PATH"]}', EVENTS=str(self.events))
        self.mock('systemctl', '#!/bin/sh\nprintf \'%s\\n\' "$*" >> "$EVENTS"\n')
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


class ObservationDeploymentTests(unittest.TestCase):
 def test_provisioning_is_scoped_and_preserves_existing_tokens(self):
  with tempfile.TemporaryDirectory() as d:
   out=pathlib.Path(d)/'credentials'
   cmd=['python3',str(ROOT/'packaging/gcp/provision-observability.py'),'--output',str(out),'--endpoint','https://example.com/observability','--rover','4','--producer','phone-unit']
   subprocess.run(cmd,check=True,capture_output=True)
   token=(out/'phone-unit.token').read_text().strip();config=json.loads((out/'credentials.json').read_text())
   writer=next(c for c in config if c['scope']['producer_id']=='phone-unit')
   self.assertEqual(writer['token_sha256'],hashlib.sha256(token.encode()).hexdigest());self.assertEqual(writer['scope']['rovers'],['4','_producer']);self.assertFalse(writer['scope']['read'])
   self.assertNotIn(token,(out/'credentials.json').read_text());self.assertEqual((out/'phone-unit.token').stat().st_mode&0o777,0o600)
   subprocess.run(cmd,check=True,capture_output=True);self.assertEqual((out/'phone-unit.token').read_text().strip(),token)
 def test_service_is_independent_and_loopback_only(self):
  service=(ROOT/'packaging/gcp/terra-observability.service').read_text()
  self.assertIn('User=terra-observability',service);self.assertIn('UMask=0077',service)
  self.assertIn('TERRA_OBSERVABILITY_BIND=127.0.0.1:7450',service)
  self.assertIn('ProtectSystem=strict',service);self.assertNotIn('User=argos\n',service)
  snippet=(ROOT/'packaging/gcp/argos-observability.caddy').read_text();self.assertIn('/observability/*',snippet);self.assertIn('127.0.0.1:7450',snippet)
if __name__=='__main__':unittest.main()

class ObservationInstallerTests(unittest.TestCase):
 def test_install_adds_collector_without_touching_router_auth_or_tls(self):
  fixture=ObservationHost();fixture.setUp()
  try:
   root=fixture.root;release=root/'release'
   (release/'bin/terra-observability-collector').write_text('collector')
   (release/'observability').mkdir();(release/'observability/credentials.json').write_text('[]')
   (release/'replay').mkdir();(release/'replay/uv.lock').write_text('lock')
   fixture.mock('uv','#!/bin/sh\nexit 0\n')
   auth=root/'etc/argos/fleet.password';auth.write_text('existing-secret')
   tls=root/'var/lib/argos/tls';tls.mkdir();(tls/'server.key').write_text('existing-key')
   source=(ROOT/'packaging/gcp/install-observability.sh').read_text()
   for prefix in ['/tmp/argos-release','/opt/argos','/opt/terra-observability','/etc/terra-observability','/var/lib/terra-observability','/etc/systemd/system','/etc/caddy']:
    destination=release if prefix=='/tmp/argos-release' else root/prefix.lstrip('/')
    source=source.replace(prefix,str(destination))
   subprocess.run(['bash','-c',source],env=fixture.env,check=True,capture_output=True)
   self.assertEqual(auth.read_text(),'existing-secret');self.assertEqual((tls/'server.key').read_text(),'existing-key')
   self.assertTrue((root/'etc/systemd/system/terra-observability.service').exists())
   self.assertIn('enable --now terra-observability',fixture.events.read_text());self.assertNotIn('restart argos-router',fixture.events.read_text())
  finally:fixture.doCleanups()
