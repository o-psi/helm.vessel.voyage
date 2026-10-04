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
        require(len(record['dependency_fingerprints']) == len(cargo['deps']),
                'missing dependency fingerprint closure')
        require(not cargo['deps'], 'dependency closure needs native recursive qualification; refusing')
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
