"""Source-owned, conservative rollback contract for managed system releases.

Equality requires identical execution/persistence/protocol implementations and
build inputs. This deliberately refuses a guessed cross-schema compatibility
promise. A changed implementation uses reviewed quiescent forward adoption.
"""
import hashlib
import json
from pathlib import Path
import re
import tomllib

MEMBERS = {'helm', 'vessel', 'voyage', 'voyage-installer', 'voyage-protocol', 'voyage-storage'}

def _hash(items):
    value = hashlib.sha256()
    for name, data in sorted(items):
        value.update(name.encode() + b'\0' + data + b'\0')
    return value.hexdigest()

def _constant(path, name):
    match = re.search(r'const\s+' + re.escape(name) + r'\s*:\s*[^=;]+?=\s*([0-9]+)\s*;', path.read_text())
    if not match:
        raise ValueError(f'Missing code-owned format constant: {name}')
    return int(match[1])

def _schemas(path, name):
    match = re.search(r'const\s+' + name + r'\s*:\s*&\[i64\]\s*=\s*&\[([0-9,\s]+)\]', path.read_text())
    if not match:
        raise ValueError(f'Missing actual catalogue parser contract: {name}')
    values = [int(value.strip()) for value in match[1].split(',') if value.strip()]
    if values != sorted(set(values)) or not values:
        raise ValueError(f'Invalid catalogue parser contract: {name}')
    return values

def contract(source):
    source = Path(source)
    database = source / 'vessel/src/process/database.rs'
    journal = source / 'voyage/src/attachment/journal.rs'
    journal_schema = _constant(journal, 'SCHEMA_VERSION')
    # The live parser's explicit read list ends in its current writer constant.
    reader = re.search(r'matches!\(\s*version,\s*([0-9|\s]+)\|\s*SCHEMA_VERSION\s*\)', journal.read_text())
    if not reader:
        raise ValueError('Journal reader contract is not the known explicit parser')
    journal_read = sorted(set([int(x.strip()) for x in reader[1].split('|') if x.strip()] + [journal_schema]))
    formats = {
        'catalogue_read': _schemas(database, 'CATALOGUE_READ_SCHEMAS'),
        'catalogue_write': _schemas(database, 'CATALOGUE_WRITE_SCHEMAS'),
        'journal_read': journal_read,
        'journal_write': [journal_schema],
        'process_protocol': [_constant(source / 'crates/voyage-protocol/src/process/types.rs', 'PROCESS_PROTOCOL')],
        'vessel_protocol': [_constant(source / 'crates/voyage-protocol/src/vessel.rs', 'VESSEL_API_VERSION')],
        'execution_identity': [_constant(source / 'crates/voyage-protocol/src/execution_identity.rs', 'EXECUTION_SCHEMA')],
    }
    implementations = []
    for directory in ('vessel/src', 'voyage/src', 'crates/voyage-protocol/src', 'crates/voyage-storage/src', 'installer/src'):
        for path in sorted((source / directory).rglob('*')):
            if path.is_file() and path.suffix in ('.rs', '.sql'):
                implementations.append((str(path.relative_to(source)), path.read_bytes()))
    if not implementations:
        raise ValueError('No execution format implementation was inventoried')
    inputs = []
    for name in ('Cargo.toml', 'helm/Cargo.toml', 'vessel/Cargo.toml', 'voyage/Cargo.toml', 'installer/Cargo.toml', 'crates/voyage-protocol/Cargo.toml', 'crates/voyage-storage/Cargo.toml'):
        value = tomllib.loads((source / name).read_text())
        if name == 'Cargo.toml':
            value.get('workspace', {}).get('package', {}).pop('version', None)
        else:
            value.get('package', {}).pop('version', None)
        inputs.append((name, json.dumps(value, sort_keys=True, separators=(',', ':')).encode()))
    lock = tomllib.loads((source / 'Cargo.lock').read_text())
    for package in lock.get('package', []):
        if package['name'] in MEMBERS and 'source' not in package:
            package.pop('version', None)
    inputs.append(('Cargo.lock', json.dumps(lock, sort_keys=True, separators=(',', ':')).encode()))
    return {'schema_version': 1, 'formats': formats,
            'implementation_sha256': _hash(implementations), 'build_inputs_sha256': _hash(inputs)}
