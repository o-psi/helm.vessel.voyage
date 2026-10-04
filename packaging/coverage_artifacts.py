#!/usr/bin/env python3
"""Select current workspace coverage objects from Cargo JSON, never a deps glob.

Offline evidence tool: does not build, clean, merge profiles or change exclusions.
"""
import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path


def require(ok, message):
    if not ok:
        raise ValueError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_inventory(root):
    # Include manifests and all Rust source, including untracked/generated source
    # outside target. Source changes invalidate the selection, even dirty ones.
    inventory = {}
    for directory, dirs, files in os.walk(root):
        dirs[:] = sorted(d for d in dirs if d not in ('target', '.git', '.local-git'))
        for name in sorted(files):
            if name.endswith('.rs') or name in ('Cargo.toml', 'Cargo.lock'):
                p = Path(directory) / name
                inventory[str(p.relative_to(root))] = digest(p)
    return inventory


def select(metadata, messages, root):
    members = set(metadata['workspace_members'])
    packages = {p['id']: p for p in metadata['packages'] if p['id'] in members}
    require(set(packages) == members, 'metadata missing workspace members')
    require(Path(metadata['workspace_root']).resolve() == root,
            'metadata belongs to another checkout')
    expected = {(p['id'], t['name'], tuple(t['kind']))
                for p in packages.values() for t in p['targets']
                if t.get('test', True) and 'custom-build' not in t['kind']}
    seen, objects, tests = set(), {}, set()
    finished = False
    target_root = Path(metadata.get('target_directory', root / 'target')).resolve()
    for m in messages:
        if m.get('reason') == 'build-finished':
            require(m['success'], 'Cargo build failed')
            finished = True
        if m.get('reason') != 'compiler-artifact' or m['package_id'] not in members:
            continue
        t = m['target']
        key = (m['package_id'], t['name'], tuple(t['kind']))
        if m['profile']['test']:
            seen.add(key)
        executable = m.get('executable')
        if executable:
            path = Path(executable).resolve()
            require(path.is_relative_to(target_root), 'executable outside Cargo target directory')
            require(path.is_file(), 'missing current executable: ' + str(path))
            objects[str(path)] = digest(path)
            if m['profile']['test']:
                tests.add(str(path))
    require(finished, 'missing successful build-finished record')
    require(expected <= seen, 'missing workspace test targets: ' + repr(sorted(expected-seen)))
    require(objects and tests, 'empty workspace executable/test selection')
    return {'schema': 1, 'workspace': str(root), 'sources': source_inventory(root),
            'objects': objects, 'test_objects': sorted(tests),
            'workspace_members': sorted(members),
            'targets': [list(k[:2]) + [list(k[2])] for k in sorted(seen)],
            'features': {p: sorted({f for m in messages
                        if m.get('reason') == 'compiler-artifact' and m.get('package_id') == p
                        for f in m.get('features', [])}) for p in sorted(members)}}


def audit(manifest, detail, diagnostics, root):
    require(manifest['workspace'] == str(root), 'manifest checkout changed')
    require(manifest['sources'] == source_inventory(root), 'measured source changed')
    for name, sha in manifest['objects'].items():
        require(Path(name).is_file() and digest(Path(name)) == sha,
                'selected object changed: ' + name)
    require(not diagnostics.strip(), 'LLVM diagnostics present; retain and investigate')
    require(len(detail['data']) == 1, 'expected one LLVM export dataset')
    logical = set()
    for f in detail['data'][0]['files']:
        p = Path(f['filename']).resolve()
        # Rust std/registry paths should have been excluded by the existing
        # llvm-cov default regex, supplied explicitly at export time.
        require(p.is_relative_to(root), 'foreign source mapping: ' + str(p))
        name = str(p.relative_to(root))
        require(name in manifest['sources'], 'unknown source mapping: ' + name)
        require(name not in logical, 'duplicate logical source: ' + name)
        logical.add(name)
    require(logical, 'empty source mapping inventory')
    return {'objects': len(manifest['objects']), 'mapped_sources': len(logical),
            'source_fingerprint': hashlib.sha256(json.dumps(manifest['sources'],
                                    sort_keys=True).encode()).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    s = sub.add_parser('select')
    s.add_argument('--metadata', type=Path, required=True)
    s.add_argument('--cargo-json', type=Path, required=True)
    s.add_argument('--output', type=Path, required=True)
    e = sub.add_parser('export')
    e.add_argument('--manifest', type=Path, required=True)
    e.add_argument('--llvm-cov', required=True)
    e.add_argument('--profile', type=Path, required=True)
    e.add_argument('--ignore-filename-regex', required=True)
    e.add_argument('--output', type=Path, required=True)
    e.add_argument('--diagnostics', type=Path, required=True)
    a = sub.add_parser('audit')
    a.add_argument('--manifest', type=Path, required=True)
    a.add_argument('--export', type=Path, required=True)
    a.add_argument('--diagnostics', type=Path, required=True)
    for p in (s, e, a):
        p.add_argument('--source-root', type=Path, required=True)
    args = parser.parse_args()
    root = args.source_root.resolve()
    if args.command == 'select':
        messages = [json.loads(line) for line in args.cargo_json.read_text().splitlines() if line.strip()]
        result = select(json.loads(args.metadata.read_text()), messages, root)
        # Exclusive creation preserves earlier evidence.
        with args.output.open('x') as out:
            json.dump(result, out, indent=2, sort_keys=True)
            out.write('\n')
    elif args.command == 'export':
        manifest = json.loads(args.manifest.read_text())
        # Validate unchanged input before LLVM; a nonempty placeholder export
        # lets audit enforce the same source/object constraints.
        source = next(iter(manifest['sources']))
        audit(manifest, {'data': [{'files': [{'filename': str(root / source)}]}]}, '', root)
        require(args.profile.is_file(), 'missing explicitly merged current profile')
        profile_sha = digest(args.profile)
        command = [args.llvm_cov, 'export', '-instr-profile=' + str(args.profile),
                   '-ignore-filename-regex=' + args.ignore_filename_regex]
        for obj in manifest['objects']:
            command.extend(['-object', obj])
        with args.output.open('x') as out, args.diagnostics.open('x') as err:
            result = subprocess.run(command, stdout=out, stderr=err, check=False, timeout=600)
        require(digest(args.profile) == profile_sha, 'merged profile changed during export')
        require(result.returncode == 0, 'LLVM export failed; preserve evidence')
        print(json.dumps(audit(manifest, json.loads(args.output.read_text()),
                              args.diagnostics.read_text(), root), sort_keys=True))
    else:
        print(json.dumps(audit(json.loads(args.manifest.read_text()),
                              json.loads(args.export.read_text()),
                              args.diagnostics.read_text(), root), sort_keys=True))


if __name__ == '__main__':
    main()
