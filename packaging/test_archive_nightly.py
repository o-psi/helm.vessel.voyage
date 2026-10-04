"""Offline target packaging fixtures; not native installation evidence."""
import hashlib
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile

import archive_nightly
import stage_nightly


class NightlyArchiveTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.browser = self.root / 'browser'
        for name, data in {
            'worker.mjs': 'export {};', 'guardian.py': '# guardian',
            'mirror-source.mjs': 'export {};', 'rrweb-vendor.mjs': 'export {};',
            'rrweb-LICENSE': 'MIT',
            'package.json': '{"dependencies":{"playwright-core":"1.63.0"}}',
            'package-lock.json': '{}',
            'node_modules/playwright-core/package.json': '{"version":"1.63.0"}',
        }.items():
            path = self.browser / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(data)
        self.binaries = self.root / 'bin'
        self.binaries.mkdir()
        for suffix in ('', '.exe'):
            for binary in stage_nightly.BINARIES:
                (self.binaries / (binary + suffix)).write_bytes(b'fixture, not executable')
        self.version = '1.0.3-nightly.20261004.123.1'

    def build(self, target):
        return archive_nightly.archive(self.binaries, self.root / 'output', self.version,
                                       'a' * 40, target, self.browser)

    def test_all_targets_exact_inventory_and_identity(self):
        for target, suffix in stage_nightly.TARGETS.items():
            with self.subTest(target=target):
                path = self.build(target)
                checksum = Path(str(path) + '.sha256').read_text()
                self.assertEqual(checksum, hashlib.sha256(path.read_bytes()).hexdigest() + '  ' + path.name + '\n')
                if suffix:
                    with zipfile.ZipFile(path) as bundle:
                        entries = {name: bundle.read(name) for name in bundle.namelist()}
                else:
                    with tarfile.open(path) as bundle:
                        entries = {member.name: bundle.extractfile(member).read()
                                   for member in bundle.getmembers() if member.isfile()}
                prefix = f'voyage-{self.version}-{target}/'
                manifest = json.loads(entries[prefix + 'release.json'])
                self.assertEqual(manifest['target'], target)
                self.assertEqual(manifest['version'], self.version)
                self.assertEqual(set(manifest['binaries']), {name + suffix for name in stage_nightly.BINARIES})
                for name, info in manifest['binaries'].items():
                    self.assertEqual(info['sha256'], hashlib.sha256(entries[prefix + 'bin/' + name]).hexdigest())
                for name, info in manifest['assets'].items():
                    self.assertEqual(info['sha256'], hashlib.sha256(entries[prefix + name]).hexdigest())
                self.assertIn(('Source: ' + 'a' * 40).encode(), entries[prefix + 'BUILD.txt'])
                self.assertIn(b'does not establish native', entries[prefix + 'BUILD.txt'])
                with self.assertRaisesRegex(ValueError, 'overwrite'):
                    self.build(target)

    def test_invalid_inputs_publish_nothing(self):
        for version, source, target in [('1.0.3', 'a' * 40, 'x86_64-apple-darwin'),
                                        (self.version, 'main', 'x86_64-apple-darwin'),
                                        (self.version, 'a' * 40, 'unknown')]:
            with self.assertRaises(ValueError):
                archive_nightly.archive(self.binaries, self.root / 'output', version, source, target, self.browser)
        self.assertFalse((self.root / 'output').exists())

    def test_missing_binary_and_browser_refuse_before_publication(self):
        (self.binaries / 'vessel.exe').unlink()
        with self.assertRaises(ValueError):
            self.build('x86_64-pc-windows-msvc')
        (self.browser / 'worker.mjs').unlink()
        with self.assertRaises(ValueError):
            self.build('x86_64-apple-darwin')
        self.assertEqual(list((self.root / 'output').iterdir()), [])


if __name__ == '__main__':
    unittest.main()
