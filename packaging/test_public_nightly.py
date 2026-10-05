"""Offline public-nightly acquisition checks using real archives and mocked HTTPS."""
import ast
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
VERSION = '1.0.2-nightly.20260927.123.1'
SOURCE = 'a' * 40
TARGET = 'x86_64-unknown-linux-gnu'
NAME = f'voyage-{VERSION}-{TARGET}'
ASSET = NAME + '.tar.gz'
API = 'https://api.github.com/repos/o-psi/helm.vessel.voyage/releases?per_page=100'
PINNED_API = f'https://api.github.com/repos/o-psi/helm.vessel.voyage/releases/tags/nightly-{VERSION}'
BASE = f'https://github.com/o-psi/helm.vessel.voyage/releases/download/nightly-{VERSION}/'


class PublicNightlyTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.requests = []
        self.files = {}
        self.release = dict(tag_name='nightly-' + VERSION, target_commitish=SOURCE,
                            prerelease=True, draft=False, assets=[])
        self.make_archive()
        # Load the worker's definitions without executing its command-line entry.
        tree = ast.parse((ROOT / 'installer/src/source_acquire.py').read_text())
        tree.body = [n for n in tree.body if isinstance(n, (ast.Import, ast.ImportFrom, ast.FunctionDef, ast.ClassDef))]
        self.worker = dict(ROOT=self.root, AUTHENTICATED=False)
        exec(compile(tree, 'source_acquire.py', 'exec'), self.worker)
        self.worker['fetch'] = self.fetch

    def make_archive(self, source=SOURCE, version=VERSION, browser=True, unsafe=False):
        installer = b'#!/bin/sh\nprintf "%s\\n" "$@" > "$ARGS_LOG"\n'
        entries = {f'bin/{n}': installer for n in ('helm', 'vessel', 'voyage', 'voyage-installer')}
        manifest = dict(schema_version=1, version=version, target=TARGET,
                        binaries={n: dict(sha256=hashlib.sha256(b).hexdigest()) for n, b in
                                  ((k[4:], v) for k, v in entries.items())}, assets={})
        if browser:
            entries['share/voyage/browser/worker.mjs'] = b'// fixture'
            manifest['assets']['share/voyage/browser/worker.mjs'] = dict(sha256=hashlib.sha256(b'// fixture').hexdigest())
        entries['release.json'] = json.dumps(manifest).encode()
        entries['BUILD.txt'] = f'Source: {source}\n'.encode()
        if unsafe:
            entries['../escape'] = b'no'
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode='w:gz') as archive:
            for name, content in entries.items():
                member = tarfile.TarInfo(f'{NAME}/{name}')
                member.size = len(content)
                archive.addfile(member, io.BytesIO(content))
        self.files[BASE + ASSET] = buf.getvalue()
        self.files[BASE + ASSET + '.sha256'] = f'{hashlib.sha256(buf.getvalue()).hexdigest()}  {ASSET}\n'.encode()
        self.release['assets'] = [dict(name=n, size=len(self.files[BASE + n])) for n in (ASSET, ASSET + '.sha256')]

    def fetch(self, url, path, label, limit):
        self.requests.append(url)
        if url == API:
            data = json.dumps([self.release]).encode()
        elif url == PINNED_API:
            data = json.dumps(self.release).encode()
        else:
            data = self.files[url]
        self.assertLessEqual(len(data), limit)
        path.write_bytes(data)

    def acquire(self):
        return self.worker['nightly'](TARGET)

    def acquire_pinned(self, version=VERSION, target=TARGET):
        return self.worker['pinned_nightly'](target, version)

    def test_anonymous_full_archive(self):
        binaries, description = self.acquire()
        self.assertTrue((binaries.parent / 'share/voyage/browser/worker.mjs').is_file())
        self.assertIn(SOURCE, description)
        self.assertEqual(set(self.requests), {API, BASE + ASSET, BASE + ASSET + '.sha256'})

    def test_exact_public_tag_retains_older_version_without_latest_lookup_or_execution(self):
        for asset in self.release['assets']:
            asset['browser_download_url'] = 'https://untrusted.example/archive'
        binaries, description = self.acquire_pinned()
        self.assertIn(VERSION, description)
        self.assertIn(SOURCE, description)
        self.assertEqual(self.requests, [PINNED_API, BASE + ASSET, BASE + ASSET + '.sha256'])
        self.assertTrue((binaries.parent / 'share/voyage/browser/worker.mjs').is_file())
        self.assertFalse((self.root / 'args').exists())

    def test_exact_tag_rejects_invalid_versions_before_network(self):
        for version in ('', 'nightly-' + VERSION, 'v' + VERSION, '1.0.2',
                        '1.0.2-nightly.2026092.123.1', '1.0.2-nightly.202609270.123.1',
                        '1.0.2-nightly.20260927..1', VERSION + '/../../latest',
                        VERSION + '\n', VERSION.replace('123', '\u0661\u0662\u0663'),
                        '1' * 129 + '.0.2-nightly.20260927.123.1'):
            with self.subTest(version=version):
                with self.assertRaisesRegex(self.worker['Failure'], 'Invalid pinned'):
                    self.acquire_pinned(version)
        self.assertEqual(self.requests, [])

    def test_exact_tag_rejects_unsupported_target_before_network(self):
        with self.assertRaisesRegex(self.worker['Failure'], 'Linux x86-64'):
            self.acquire_pinned(target='aarch64-unknown-linux-gnu')
        self.assertEqual(self.requests, [])

    def test_exact_tag_missing_release_never_falls_back(self):
        def missing(url, path, label, limit):
            self.requests.append(url)
            raise self.worker['Failure']('Exact tag unavailable')
        self.worker['fetch'] = missing
        with self.assertRaisesRegex(self.worker['Failure'], 'Exact tag unavailable'):
            self.acquire_pinned()
        self.assertEqual(self.requests, [PINNED_API])

    def test_exact_tag_rejects_changed_release_metadata_without_asset_downloads(self):
        for changes in ({'draft': True}, {'prerelease': False},
                        {'tag_name': 'nightly-1.0.2-nightly.20260928.124.1'},
                        {'tag_name': None}, {'draft': 0}, {'prerelease': 1}):
            with self.subTest(changes=changes), patch.dict(self.release, changes):
                with self.assertRaisesRegex(self.worker['Failure'], 'metadata mismatch'):
                    self.acquire_pinned()
        self.assertTrue(all(url == PINNED_API for url in self.requests))

    def test_exact_tag_rejects_nonobject_metadata(self):
        def invalid(url, path, label, limit):
            self.requests.append(url)
            path.write_text('[]')
        self.worker['fetch'] = invalid
        with self.assertRaisesRegex(self.worker['Failure'], 'metadata mismatch'):
            self.acquire_pinned()
        self.assertEqual(self.requests, [PINNED_API])

    def test_exact_tag_missing_assets_refused(self):
        self.release['assets'] = []
        with self.assertRaisesRegex(self.worker['Failure'], 'asset missing'):
            self.acquire_pinned()
        self.assertEqual(self.requests, [PINNED_API])

    def test_exact_tag_requires_exact_source_commit(self):
        self.release['target_commitish'] = 'main'
        with self.assertRaisesRegex(self.worker['Failure'], 'exact source'):
            self.acquire_pinned()
        self.assertEqual(self.requests, [PINNED_API])

    def test_exact_tag_checksum_mismatch_refused_before_extraction(self):
        self.files[BASE + ASSET + '.sha256'] = f'{"0" * 64}  {ASSET}\n'.encode()
        with self.assertRaisesRegex(self.worker['Failure'], 'checksum mismatch'):
            self.acquire_pinned()
        self.assertFalse((self.root / NAME).exists())

    def test_exact_tag_source_mismatch_refused(self):
        self.make_archive(source='b' * 40)
        with self.assertRaisesRegex(self.worker['Failure'], 'inventory mismatch'):
            self.acquire_pinned()

    def test_exact_tag_version_mismatch_refused(self):
        self.make_archive(version='1.0.2-nightly.20260928.124.1')
        with self.assertRaisesRegex(self.worker['Failure'], 'inventory mismatch'):
            self.acquire_pinned()

    def test_exact_tag_browser_missing_refused(self):
        self.make_archive(browser=False)
        with self.assertRaisesRegex(self.worker['Failure'], 'inventory mismatch'):
            self.acquire_pinned()

    def test_exact_tag_unsafe_archive_refused(self):
        self.make_archive(unsafe=True)
        with self.assertRaisesRegex(self.worker['Failure'], 'Unsafe'):
            self.acquire_pinned()
        self.assertFalse((self.root / 'escape').exists())

    def test_absent_draft_or_stable_refused(self):
        for changes in ({'draft': True}, {'prerelease': False}, {'tag_name': 'v1.0.1'}):
            with self.subTest(changes=changes), patch.dict(self.release, changes):
                with self.assertRaisesRegex(self.worker['Failure'], 'No public nightly'):
                    self.acquire()

    def test_missing_asset_refused(self):
        self.release['assets'] = []
        with self.assertRaisesRegex(self.worker['Failure'], 'asset missing'):
            self.acquire()

    def test_moving_source_refused(self):
        self.release['target_commitish'] = 'main'
        with self.assertRaisesRegex(self.worker['Failure'], 'exact source'):
            self.acquire()

    def test_checksum_mismatch_refused(self):
        self.files[BASE + ASSET + '.sha256'] = f'{"0" * 64}  {ASSET}\n'.encode()
        with self.assertRaisesRegex(self.worker['Failure'], 'checksum mismatch'):
            self.acquire()

    def test_source_mismatch_refused(self):
        self.make_archive(source='b' * 40)
        with self.assertRaisesRegex(self.worker['Failure'], 'inventory mismatch'):
            self.acquire()

    def test_version_mismatch_refused(self):
        self.make_archive(version='1.0.1')
        with self.assertRaisesRegex(self.worker['Failure'], 'inventory mismatch'):
            self.acquire()

    def test_browser_missing_refused(self):
        self.make_archive(browser=False)
        with self.assertRaisesRegex(self.worker['Failure'], 'inventory mismatch'):
            self.acquire()

    def test_unsafe_archive_refused(self):
        self.make_archive(unsafe=True)
        with self.assertRaisesRegex(self.worker['Failure'], 'Unsafe'):
            self.acquire()
        self.assertFalse((self.root / 'escape').exists())

    def bootstrap(self, *args, version='nightly'):
        # Execute the complete standalone bootstrap with fixture curl/systemd/id;
        # Python/extraction/hashes/selection are real. No credentials or network.
        bin_dir = self.root / 'tools'
        bin_dir.mkdir(exist_ok=True)
        fixture = self.root / 'https.json'
        fixture.write_text(json.dumps({API: json.dumps([self.release]), **{k: v.hex() for k, v in self.files.items()}}))
        curl = f'''#!{sys.executable}
import json, pathlib, sys
args = sys.argv[1:]
assert '--disable' in args
assert not any(s in args for s in ['--header', '-H', '--user'])
data = json.loads(pathlib.Path({str(fixture)!r}).read_text())[args[-1]]
pathlib.Path(args[args.index('--output') + 1]).write_bytes(data.encode() if args[-1] == {API!r} else bytes.fromhex(data))
'''
        python = f'''#!{sys.executable}
import os, sys, tempfile
os.confstr = lambda key: 'glibc 2.39'
tempfile.mkdtemp = lambda **kwargs: {str(self.root / 'download')!r}
if sys.argv[1] == '-c': exec(sys.argv[2])
else:
    sys.argv = sys.argv[1:]
    exec(sys.stdin.read())
'''
        for name, text in dict(curl=curl, python3=python, id='#!/bin/sh\necho 1000\n',
                               uname='#!/bin/sh\ncase "$1" in -s) echo Linux;; *) echo x86_64;; esac\n',
                               systemctl='#!/bin/sh\nexit 0\n', gh='#!/bin/sh\nexit 98\n').items():
            p = bin_dir / name
            p.write_text(text)
            p.chmod(0o700)
        (self.root / 'download').mkdir(exist_ok=True)
        # Safe home under real HOME; the fixture overrides only mkdtemp destination.
        env = {k: v for k, v in os.environ.items() if not k.startswith(('VOYAGE_', 'GH_', 'GITHUB_'))}
        env.update(PATH=f'{bin_dir}:' + os.environ['PATH'], VOYAGE_VERSION=version, ARGS_LOG=str(self.root / 'args'))
        return subprocess.run(['sh', str(ROOT / 'install.sh'), *(args or ('install', '--no-start'))], env=env,
                              capture_output=True, text=True, timeout=15)

    def test_bootstrap_anonymous(self):
        result = self.bootstrap()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.root / 'args').read_text().splitlines()[-2:], ['install', '--no-start'])

    def test_bootstrap_dev_uses_pinned_nightly_without_source_build(self):
        result = self.bootstrap('upgrade', '--dev', '--start', version='latest')
        self.assertEqual(result.returncode, 0, result.stderr)
        args = (self.root / 'args').read_text().splitlines()
        self.assertEqual(args[-2:], ['upgrade', '--start'])
        self.assertEqual(args[0], '--bin-dir')
        self.assertNotIn('--dev', args)

    def test_bootstrap_dev_refuses_stable_pin(self):
        result = self.bootstrap('upgrade', '--dev', version='v1.0.1')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('requires a public nightly', result.stderr)
        self.assertFalse((self.root / 'args').exists())

    def test_bootstrap_source_mismatch(self):
        self.make_archive(source='b' * 40)
        result = self.bootstrap()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('source identity mismatch', result.stderr)
        self.assertFalse((self.root / 'args').exists())

    def test_bootstrap_no_nightly(self):
        self.release['prerelease'] = False
        result = self.bootstrap()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('No public nightly', result.stderr)
        self.assertFalse((self.root / 'args').exists())

    def test_bootstrap_missing_asset(self):
        self.release['assets'] = []
        result = self.bootstrap()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('asset missing', result.stderr)


if __name__ == '__main__':
    unittest.main()
