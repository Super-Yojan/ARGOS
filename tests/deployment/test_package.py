"""Lay out release packages from a stand-in binary; never build the app."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]


class PackageTests(unittest.TestCase):
    def package(self, platform, *extra):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        out = Path(tmp.name)
        binary = out / 'argos-gpui'
        binary.write_bytes(b'stand-in')
        subprocess.run(
            [sys.executable, str(ROOT / 'scripts/package.py'), '--binary', str(binary),
             '--platform', platform, '--version', '1.2.3', '--out', str(out), *extra],
            check=True, capture_output=True,
        )
        return out

    def test_linux_zip_has_the_executable_assets_and_env_example(self):
        out = self.package('linux-x64')
        with zipfile.ZipFile(out / 'argos-1.2.3-linux-x64.zip') as archive:
            names = set(archive.namelist())
            mode = archive.getinfo('argos-1.2.3-linux-x64/argos').external_attr >> 16
        self.assertTrue({
            'argos-1.2.3-linux-x64/argos',
            'argos-1.2.3-linux-x64/assets/robot.glb',
            'argos-1.2.3-linux-x64/.env.example',
            'argos-1.2.3-linux-x64/README.txt',
            'argos-1.2.3-linux-x64/NOTICE.txt',
        } <= names)
        self.assertTrue(mode & 0o111)

    def test_windows_executable_is_argos_exe(self):
        out = self.package('windows-x64')
        with zipfile.ZipFile(out / 'argos-1.2.3-windows-x64.zip') as archive:
            self.assertIn('argos-1.2.3-windows-x64/argos.exe', archive.namelist())

    def test_macos_bundle_keeps_assets_in_resources(self):
        out = self.package('macos-arm64')
        app = out / 'argos-1.2.3-macos-arm64/ARGOS.app/Contents'
        self.assertTrue((app / 'MacOS/argos').is_file())
        self.assertTrue((app / 'Resources/assets/robot.glb').is_file())
        self.assertIn('<string>1.2.3</string>', (app / 'Info.plist').read_text())
        self.assertFalse((out / 'argos-1.2.3-macos-arm64.zip').exists())


if __name__ == '__main__':
    unittest.main()
