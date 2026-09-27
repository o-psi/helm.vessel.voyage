"""Offline nightly trust/size/membership checks; no GitHub or provider access."""
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

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
        name = f'voyage-{version}-x86_64-unknown-linux-gnu'
        tar = b'fixture verified archive'
        zipbytes = io.BytesIO()
        with zipfile.ZipFile(zipbytes, 'w') as archive:
            archive.writestr(name + '.tar.gz', tar)
            archive.writestr(name + '.tar.gz.sha256', hashlib.sha256(tar).hexdigest() + '  ' + name + '.tar.gz')
            if changes.get('extra'):
                archive.writestr('../escape', 'unsafe')
        artifact = dict(id=12, name=f'nightly-{commit}-{version}', expired=False, size_in_bytes=100, created_at='2026', workflow_run={'id': 123})
        calls = []
        def api(endpoint, destination, *args, **kwargs):
            calls.append(endpoint)
            if endpoint.endswith('/zip'):
                destination.write_bytes(zipbytes.getvalue())
            else:
                data = {'artifacts': [artifact]} if 'artifacts?' in endpoint else dict(path=changes.get('workflow', '.github/workflows/nightly.yml'), head_branch='main', status='completed', conclusion=changes.get('conclusion', 'success'))
                destination.write_text(json.dumps(data))
        def extract(archive, expected):
            self.assertEqual(expected, name)
            binary = root / expected / 'bin'
            binary.mkdir(parents=True)
            (binary.parent / 'release.json').write_text(json.dumps(dict(version=version, target='x86_64-unknown-linux-gnu', assets={} if changes.get('assets_missing') else {'worker': {}})))
            (binary.parent / 'BUILD.txt').write_text('Source: ' + ('b' * 40 if changes.get('source_mismatch') else commit) + '\n')
            return binary
        ns.update(AUTHENTICATED=changes.get('authenticated', True), github_api=api, extract=extract)
        return ns, calls

    def test_pinned_success(self):
        ns, calls = self.fixture()
        _, description = ns['nightly']('x86_64-unknown-linux-gnu')
        self.assertIn('source ' + 'a' * 40, description)
        self.assertIn('workflow 123', description)
        self.assertEqual(len(calls), 3)
        self.assertTrue(all(path.startswith('repos/o-psi/helm.vessel.voyage/') for path in calls))

    def test_refuses_untrusted_or_incomplete_artifacts(self):
        for scenario in [dict(authenticated=False), dict(workflow='.github/workflows/other.yml'), dict(conclusion='failure'), dict(extra=True), dict(source_mismatch=True), dict(assets_missing=True)]:
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
