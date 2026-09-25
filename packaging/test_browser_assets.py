import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import browser_assets


class BrowserAssets(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'source'
        self.source.mkdir()
        self.dest = self.root / 'dest'
        self.dest.mkdir()
        for name, data in {
            'worker.mjs': 'export {};',
            'guardian.py': '# guardian',
            'mirror-source.mjs': 'export {};',
            'rrweb-vendor.mjs': 'export {};',
            'rrweb-LICENSE': 'MIT License',
            'package.json': json.dumps({'dependencies': {'playwright-core': '1.63.0'}}),
            'package-lock.json': '{}',
            'node_modules/playwright-core/package.json': '{"version":"1.63.0"}',
        }.items():
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(data)

    def test_stage_exact_hashed_members(self):
        assets = browser_assets.stage(self.source, self.dest)
        self.assertEqual(len(assets), 8)
        for name, info in assets.items():
            self.assertEqual(info['sha256'], hashlib.sha256((self.dest / name).read_bytes()).hexdigest())
            self.assertEqual((self.dest / name).stat().st_mode & 0o777, 0o644)
        with self.assertRaises(ValueError):
            browser_assets.stage(self.source, self.dest)

    def test_incomplete_or_wrong_dependency_refused(self):
        (self.source / 'node_modules/playwright-core/package.json').write_text('{"version":"0"}')
        with self.assertRaises(ValueError):
            browser_assets.stage(self.source, self.dest)
        (self.source / 'worker.mjs').unlink()
        with self.assertRaises(ValueError):
            browser_assets.stage(self.source, self.dest)

    def test_symlink_and_bounds_refused(self):
        (self.source / 'mirror-source.mjs').unlink()
        (self.source / 'mirror-source.mjs').symlink_to(self.source / 'worker.mjs')
        with self.assertRaises(ValueError):
            browser_assets.stage(self.source, self.dest)

    def test_nonbrowser_source_remains_compatible(self):
        self.assertEqual(browser_assets.stage(self.root / 'absent', self.dest), {})


if __name__ == '__main__':
    unittest.main()
