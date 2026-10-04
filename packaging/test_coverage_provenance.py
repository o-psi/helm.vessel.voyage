import json
from pathlib import Path
import tempfile
import unittest
from coverage_provenance import dependencies, validate, sha


class ProvenanceTests(unittest.TestCase):
    def test_escaped_dep_info(self):
        self.assertEqual(dependencies('bin: source\\ name.rs \\\n input.sql\n'), {'source name.rs', 'input.sql'})

    def test_refusals_and_embedded_input(self):
        with tempfile.TemporaryDirectory() as directory:
            r = Path(directory)
            source = r / 'input.sql'; source.write_text('SELECT 1')
            obj = r / 'binary'; obj.write_bytes(b'object')
            dep = r / 'binary.d'; dep.write_text(f'{obj}: {source}\n')
            fp = r / 'fingerprint.json'; fp.write_text(json.dumps({'deps': [], 'rustflags': []}))
            manifest = {'objects': {str(obj): sha(obj)}, 'sources': {'input.sql': sha(source)}}
            record = {'object_sha256': sha(obj), 'dep_info': str(dep), 'dep_info_sha256': sha(dep),
                      'fingerprint': str(fp), 'fingerprint_sha256': sha(fp), 'dependency_fingerprints': []}
            evidence = {'objects': {str(obj): record}}
            self.assertEqual(validate(manifest, evidence, r), ['input.sql'])
            with self.assertRaisesRegex(ValueError, 'incomplete'):
                validate(manifest, {'objects': {}}, r)
            manifest['sources'] = {}
            with self.assertRaisesRegex(ValueError, 'unqualified generated'):
                validate(manifest, evidence, r)
            manifest['sources'] = {'input.sql': sha(source)}
            source.write_text('changed')
            with self.assertRaisesRegex(ValueError, 'input changed'):
                validate(manifest, evidence, r)
