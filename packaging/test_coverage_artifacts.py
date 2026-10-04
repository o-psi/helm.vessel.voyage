import copy
import json
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest

from coverage_artifacts import select, audit as actual_audit

def audit(manifest, detail, diagnostics, root):
    return actual_audit(manifest, detail, diagnostics, root,
                        {obj: ["lib.rs"] for obj in manifest["objects"]})


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True)
        (self.root / '.gitignore').write_text('target/\nmanifest.json\ncurrent.profdata\nllvm-fixture\narguments.json\nexport.json*\nstderr\n')
        (self.root / 'lib.rs').write_text('fn current() {}')
        self.binary = self.root / 'target' / 'test'
        self.binary.parent.mkdir()
        self.binary.write_bytes(b'object')
        self.meta = {'workspace_root': str(self.root), 'workspace_members': ['p'],
                     'packages': [{'id': 'p', 'targets': [
                         {'name': 'p', 'kind': ['lib'], 'test': True}]}]}
        self.messages = [{'reason': 'compiler-artifact', 'package_id': 'p',
                          'target': {'name': 'p', 'kind': ['lib']},
                          'profile': {'test': True}, 'features': [],
                          'executable': str(self.binary)},
                         {'reason': 'build-finished', 'success': True}]
        self.export = {'data': [{'files': [{'filename': str(self.root / 'lib.rs')}]}]}

    def manifest(self):
        return select(self.meta, self.messages, self.root)

    def test_only_current_artifacts_not_deps_glob(self):
        (self.binary.parent / 'stale').write_bytes(b'stale')
        m = self.manifest()
        self.assertEqual(list(m['objects']), [str(self.binary)])
        self.assertEqual(audit(m, self.export, '', self.root)['objects'], 1)

    def test_missing_target_and_failed_build(self):
        for messages in (self.messages[1:], self.messages[:-1],
                         [self.messages[0], {'reason': 'build-finished', 'success': False}]):
            with self.assertRaises(ValueError):
                select(self.meta, messages, self.root)

    def test_all_workspace_executables_retained(self):
        extra = self.binary.parent / 'main'
        extra.write_bytes(b'non-test executable')
        m = copy.deepcopy(self.messages[0])
        m['executable'] = str(extra)
        m['profile']['test'] = False
        self.messages.insert(0, m)
        selected = self.manifest()
        self.assertEqual(len(selected['objects']), 2)
        self.assertEqual(selected['test_objects'], [str(self.binary)])

    def test_each_member_target_required(self):
        self.meta['packages'][0]['targets'].append(
            {'name': 'integration', 'kind': ['test'], 'test': True})
        with self.assertRaisesRegex(ValueError, 'missing workspace test targets'):
            self.manifest()

    def test_missing_member(self):
        self.meta['workspace_members'].append('other')
        with self.assertRaises(ValueError):
            self.manifest()

    def test_source_and_binary_mutation(self):
        m = self.manifest()
        (self.root / 'lib.rs').write_text('fn changed() {}')
        with self.assertRaisesRegex(ValueError, 'source changed'):
            audit(m, self.export, '', self.root)
        m = self.manifest()
        self.binary.write_bytes(b'replaced')
        with self.assertRaisesRegex(ValueError, 'object changed'):
            audit(m, self.export, '', self.root)

    def test_duplicate_foreign_and_mismatched(self):
        m = self.manifest()
        bad = copy.deepcopy(self.export)
        bad['data'][0]['files'] *= 2
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            audit(m, bad, '', self.root)
        bad['data'][0]['files'] = [{'filename': '/other-checkout/lib.rs'}]
        with self.assertRaisesRegex(ValueError, 'foreign'):
            audit(m, bad, '', self.root)
        with self.assertRaisesRegex(ValueError, 'diagnostics'):
            audit(m, self.export, '27 mismatched functions', self.root)

    def test_export_cli_uses_exact_objects_and_preserves_evidence(self):
        manifest = self.root / 'manifest.json'
        manifest.write_text(json.dumps(self.manifest()))
        profile = self.root / 'current.profdata'
        profile.write_bytes(b'profile')
        tool = self.root / 'llvm-fixture'
        capture = self.root / 'arguments.json'
        tool.write_text('#!' + sys.executable + '\nimport json,sys\n'
                        + 'open(' + repr(str(capture)) + ', "w").write(json.dumps(sys.argv[1:]))\n'
                        + 'print(' + repr(json.dumps(self.export)) + ')\n')
        tool.chmod(0o700)
        output = self.root / 'export.json'
        diagnostics = self.root / 'stderr'
        dep = self.root / 'target' / 'test.d'
        dep.write_text(str(self.binary) + ': lib.rs\n')
        fp = self.root / 'target' / 'fingerprint.json'
        fp.write_text(json.dumps({'deps': [], 'rustflags': []}))
        from coverage_provenance import sha
        provenance = self.root / 'target' / 'provenance.json'
        provenance.write_text(json.dumps({'objects': {str(self.binary): {
            'object_sha256': sha(self.binary), 'dep_info': str(dep),
            'dep_info_sha256': sha(dep), 'fingerprint': str(fp),
            'fingerprint_sha256': sha(fp)}},
            'fingerprints': {str(fp): {'dependencies': []}}}))
        command = [sys.executable, str(Path(__file__).with_name('coverage_artifacts.py')),
                   'export', '--source-root', str(self.root), '--manifest', str(manifest),
                   '--input-provenance', str(provenance), '--llvm-cov', str(tool), '--profile', str(profile),
                   '--ignore-filename-regex', 'existing-default', '--output', str(output),
                   '--diagnostics', str(diagnostics)]
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        arguments = json.loads(capture.read_text())
        self.assertEqual(arguments[-2:], ['-object', str(self.binary)])
        self.assertIn('-ignore-filename-regex=existing-default', arguments)
        original = output.read_bytes()
        self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)
        self.assertEqual(output.read_bytes(), original)

    def test_embedded_nonrust_mutation(self):
        embedded = self.root / 'database.sql'
        embedded.write_text('SELECT 1')
        m = self.manifest()
        embedded.write_text('SELECT 2')
        with self.assertRaisesRegex(ValueError, 'source changed'):
            audit(m, self.export, '', self.root)

    def test_missing_ordinary_binary(self):
        self.meta['packages'][0]['targets'].append(
            {'name': 'runtime', 'kind': ['bin'], 'test': False})
        with self.assertRaisesRegex(ValueError, 'missing ordinary'):
            self.manifest()

    def test_unqualified_mapping_subset_refused(self):
        m = self.manifest()
        with self.assertRaisesRegex(ValueError, 'mapping qualification'):
            actual_audit(m, self.export, '', self.root)
        with self.assertRaisesRegex(ValueError, 'duplicate per-object'):
            actual_audit(m, self.export, '', self.root,
                         {str(self.binary): ['lib.rs', 'lib.rs']})
        with self.assertRaisesRegex(ValueError, 'incomplete object'):
            actual_audit(m, self.export, '', self.root, {})

    def test_main_worktree_switch_refused(self):
        m = self.manifest()
        other = self.root / 'other'
        other.mkdir()
        (other / 'lib.rs').write_text('fn current() {}')
        with self.assertRaisesRegex(ValueError, 'checkout changed'):
            audit(m, self.export, '', other)
        with self.assertRaisesRegex(ValueError, 'another checkout'):
            select(self.meta, self.messages, other)


if __name__ == '__main__':
    unittest.main()
