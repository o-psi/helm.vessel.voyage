import json
from pathlib import Path
import tempfile
import unittest
from coverage_provenance import dependencies, validate, sha, fingerprint_closure


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
            evidence = {'objects': {str(obj): record}, 'fingerprints': {str(fp): {'dependencies': []}}}
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

class ClosureTests(unittest.TestCase):
    def test_recursive_exact_edges_and_unknown(self):
        with tempfile.TemporaryDirectory() as directory:
            r = Path(directory)
            a, b = r / 'a.json', r / 'b.json'
            edge = [123, 'dependency', False, 456]
            a.write_text(json.dumps({'deps': [edge], 'rustflags': []}))
            b.write_text(json.dumps({'deps': [], 'rustflags': []}))
            nodes = {str(a): {'dependencies': [{'cargo_dependency': edge,
                     'path': str(b), 'sha256': sha(b)}]}, str(b): {'dependencies': []}}
            fingerprint_closure(a, sha(a), nodes)
            nodes[str(a)]['dependencies'] *= 2
            with self.assertRaises(ValueError):
                fingerprint_closure(a, sha(a), nodes)
            with self.assertRaisesRegex(ValueError, 'unknown'):
                fingerprint_closure(a, sha(a), {})

class AcquisitionTests(unittest.TestCase):
    def test_identity_acquisition_and_ambiguity(self):
        from coverage_provenance import acquire_fingerprints
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            a, b = root / 'a', root / 'b'
            a.mkdir(); b.mkdir()
            (a / 'lib-a').write_text((1).to_bytes(8, 'little').hex())
            (b / 'lib-b').write_text((2).to_bytes(8, 'little').hex())
            (a / 'lib-a.json').write_text(json.dumps({'rustflags': [], 'deps': [[9, 'b', False, 2]]}))
            (b / 'lib-b.json').write_text(json.dumps({'rustflags': [], 'deps': []}))
            nodes = acquire_fingerprints(root)
            self.assertEqual(len(nodes), 2)
            (b / 'lib-copy').write_text((2).to_bytes(8, 'little').hex())
            (b / 'lib-copy.json').write_text((b / 'lib-b.json').read_text())
            with self.assertRaisesRegex(ValueError, 'ambiguous'):
                acquire_fingerprints(root)

class BindingTests(unittest.TestCase):
    def test_missing_ambiguous_and_uninstrumented(self):
        from coverage_provenance import bind_objects
        with tempfile.TemporaryDirectory() as directory:
            obj = str(Path(directory) / 'binary')
            manifest = {'objects': {obj: 'hash'}}
            with self.assertRaisesRegex(ValueError, 'missing or ambiguous'):
                bind_objects(manifest, [], {})
            invocation = {'executable': obj, 'exit_status': 0, 'argv': ['rustc']}
            with self.assertRaisesRegex(ValueError, 'not instrumented'):
                bind_objects(manifest, [invocation], {})
            with self.assertRaisesRegex(ValueError, 'ambiguous'):
                bind_objects(manifest, [invocation, invocation], {})
