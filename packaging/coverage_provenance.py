"""Fail-closed compiler input evidence validation. No compilation or cleanup."""
import hashlib
import json
from pathlib import Path
import re
import shlex


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def dependencies(text):
    # Rust dep-info Make syntax, including escaped spaces and line continuations.
    text = text.replace('\\\n', '')
    inputs = set()
    for line in text.splitlines():
        if not line.strip() or line.startswith('#'):
            continue
        match = re.search(r'(?<!\\):\s', line)
        if match:
            inputs.update(shlex.split(line[match.end():]))
    require(inputs, 'empty or unsupported compiler dep-info')
    return inputs


def fingerprint_closure(path, expected_sha, nodes, visiting=None, done=None):
    visiting = set() if visiting is None else visiting
    done = set() if done is None else done
    path = str(Path(path).resolve())
    require(path not in visiting, 'Cargo fingerprint dependency cycle')
    if path in done:
        return
    require(sha(Path(path)) == expected_sha, 'dependency fingerprint changed')
    node = nodes.get(path)
    require(node is not None, 'unknown fingerprint producer')
    if 'marker' in node:
        require(sha(Path(node['marker'])) == node['marker_sha256'], 'producer identity marker changed')
    data = json.loads(Path(path).read_text())
    require(isinstance(data.get('deps'), list) and 'rustflags' in data,
            'unsupported Cargo fingerprint')
    edges = node.get('dependencies', [])
    require(len(edges) == len(data['deps']), 'incomplete fingerprint dependency closure')
    visiting.add(path)
    used = set()
    for dep in data['deps']:
        matches = [e for e in edges if e['cargo_dependency'] == dep]
        require(len(matches) == 1, 'unknown or ambiguous fingerprint dependency')
        edge = matches[0]
        require(edge['path'] not in used, 'duplicate fingerprint dependency producer')
        used.add(edge['path'])
        fingerprint_closure(edge['path'], edge['sha256'], nodes, visiting, done)
    visiting.remove(path)
    done.add(path)


def validate(manifest, evidence, root):
    root = Path(root).resolve()
    require(set(evidence['objects']) == set(manifest['objects']), 'incomplete input provenance objects')
    checked = set()
    for obj, record in evidence['objects'].items():
        require(record['object_sha256'] == manifest['objects'][obj] == sha(Path(obj)),
                'provenance object identity changed')
        dep = Path(record['dep_info'])
        fingerprint = Path(record['fingerprint'])
        require(sha(dep) == record['dep_info_sha256'], 'dep-info changed')
        require(sha(fingerprint) == record['fingerprint_sha256'], 'Cargo fingerprint changed')
        cargo = json.loads(fingerprint.read_text())
        require(isinstance(cargo.get('deps'), list) and 'rustflags' in cargo,
                'unsupported Cargo fingerprint')
        # Every dependency/build-script fingerprint must be recursively supplied.
        fingerprint_closure(fingerprint, record['fingerprint_sha256'],
                            evidence.get('fingerprints', {}))
        for name in dependencies(dep.read_text()):
            path = Path(name)
            if not path.is_absolute():
                path = root / path
            path = path.resolve()
            require(path.is_file(), 'missing compiler input')
            logical = str(path.relative_to(root)) if path.is_relative_to(root) else None
            if logical in manifest['sources']:
                require(sha(path) == manifest['sources'][logical], 'compiler input changed')
            else:
                # Unknown generated/ignored/external inputs cannot be qualified by
                # arbitrary operator hashes or a boolean producer attestation.
                raise ValueError('unqualified generated/ignored/external compiler input: ' + str(path))
            checked.add(logical)
        for script in record.get('build_scripts', []):
            output = Path(script['output'])
            require(sha(output) == script['sha256'], 'build-script output changed')
            for line in output.read_text().splitlines():
                value = line.removeprefix('cargo::').removeprefix('cargo:')
                if value.startswith('rerun-if-env-changed='):
                    raise ValueError('unqualified build-script environment input')
                if value.startswith('rerun-if-changed='):
                    p = (Path(script['cwd']) / value.split('=', 1)[1]).resolve()
                    require(p.is_relative_to(root) and p.is_file(), 'unqualified build-script source')
                    key = str(p.relative_to(root))
                    require(key in manifest['sources'] and sha(p) == manifest['sources'][key],
                            'unqualified build-script source')
    return sorted(checked)


def acquire_fingerprints(directory):
    """Recover exact Cargo dependency edges from adjacent identity markers.

    Cargo stores a 16-hex fingerprint next to each JSON; dep tuple's last
    integer is its little-endian u64 identity. Ambiguous identities refuse.
    All discovered nodes remain evidence, never boolean attestations.
    """
    directory = Path(directory)
    candidates = []
    for path in sorted(directory.glob('*/*.json')):
        marker = path.with_suffix('')
        if not marker.is_file():
            continue
        text = marker.read_text().strip()
        require(re.fullmatch('[0-9a-fA-F]{16}', text) is not None,
                'unsupported Cargo identity marker')
        identity = int.from_bytes(bytes.fromhex(text), 'little')
        data = json.loads(path.read_text())
        if 'deps' not in data or 'rustflags' not in data:
            continue
        candidates.append((path.resolve(), identity, data))
    require(candidates, 'no Cargo fingerprint producers')
    nodes = {}
    for path, identity, data in candidates:
        edges = []
        for dep in data['deps']:
            require(isinstance(dep, list) and len(dep) == 4 and isinstance(dep[3], int),
                    'unsupported Cargo dependency tuple')
            matches = [(p, d) for p, i, d in candidates if i == dep[3]]
            require(len(matches) == 1, 'unknown or ambiguous Cargo producer identity')
            producer = matches[0][0]
            edges.append({'cargo_dependency': dep, 'path': str(producer),
                          'sha256': sha(producer)})
        nodes[str(path)] = {'sha256': sha(path), 'identity': identity,
                            'marker': str(path.with_suffix('')),
                            'marker_sha256': sha(path.with_suffix('')),
                            'dependencies': edges}
    for path, node in nodes.items():
        fingerprint_closure(path, node['sha256'], nodes)
    return nodes


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fingerprint-directory', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    nodes = acquire_fingerprints(args.fingerprint_directory)
    with args.output.open('x') as output:
        json.dump({'fingerprints': nodes}, output, indent=2, sort_keys=True)
        output.write('\n')


if __name__ == '__main__':
    main()
