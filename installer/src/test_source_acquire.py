"""Offline public-nightly acquisition checks; no network or installer effects."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).with_name('source_acquire.py').read_text().split('\ntry:\n    execute()')[0]


class Nightly(unittest.TestCase):
    def fixture(self, **changes):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        with patch('sys.argv', ['worker', 'nightly', str(root)]):
            ns = {'__name__': 'fixture'}
            exec(compile(SOURCE, 'source_acquire.py', 'exec'), ns)
        commit = 'a' * 40
        version = '1.0.2-nightly.20260927.123.1'
        tag = 'nightly-' + version
        name = f'voyage-{version}-x86_64-unknown-linux-gnu.tar.gz'
        archive = b'fixture archive'
        release = dict(tag_name=tag, target_commitish=commit,
                       prerelease=True, draft=False,
                       assets=[dict(name=item, size=size) for item, size in
                               ((name, len(archive)), (name + '.sha256', 140))])
        release.update({key: value for key, value in changes.items()
                        if key in ('target_commitish', 'prerelease', 'draft', 'assets')})
        calls = []

        def fetch(url, destination, label, limit=536870912):
            calls.append(url)
            if url.endswith('releases?per_page=100'):
                destination.write_text(json.dumps([release]))
            elif url.endswith('.sha256'):
                digest = hashlib.sha256(archive).hexdigest()
                if changes.get('bad_checksum'):
                    digest = 'b' * 64
                destination.write_text(f'{digest}  {name}')
            else:
                destination.write_bytes(archive)

        def extract(path, expected):
            self.assertEqual(path, root / name)
            self.assertEqual(expected, name.removesuffix('.tar.gz'))
            binary = root / expected / 'bin'
            binary.mkdir(parents=True)
            manifest = dict(schema_version=1, version=version,
                            target='x86_64-unknown-linux-gnu',
                            assets={} if changes.get('assets_missing') else {'worker': {}})
            (binary.parent / 'release.json').write_text(json.dumps(manifest))
            (binary.parent / 'BUILD.txt').write_text(
                'Source: ' + ('b' * 40 if changes.get('source_mismatch') else commit) + '\n')
            return binary

        ns.update(fetch=fetch, extract=extract)
        return ns, calls

    def test_pinned_public_archive(self):
        ns, calls = self.fixture()
        _, description = ns['nightly']('x86_64-unknown-linux-gnu')
        self.assertIn('source ' + 'a' * 40, description)
        self.assertIn('archive SHA-256 ', description)
        self.assertEqual(len(calls), 3)
        self.assertTrue(all(url.startswith('https://') for url in calls))

    def test_refuses_untrusted_or_incomplete_release(self):
        for scenario in (dict(draft=True), dict(prerelease=False),
                         dict(target_commitish='main'), dict(assets=[]),
                         dict(bad_checksum=True), dict(source_mismatch=True),
                         dict(assets_missing=True)):
            with self.subTest(scenario=scenario):
                ns, _ = self.fixture(**scenario)
                with self.assertRaises(ns['Failure']):
                    ns['nightly']('x86_64-unknown-linux-gnu')

    def test_unsupported_target_does_not_contact_github(self):
        ns, calls = self.fixture()
        with self.assertRaises(ns['Failure']):
            ns['nightly']('aarch64-unknown-linux-gnu')
        self.assertEqual(calls, [])


if __name__ == '__main__':
    unittest.main()
