"""Bounded, hash-inventoried browser worker distribution (no dependency downloads)."""
from pathlib import Path
import hashlib
import json
import stat

PREFIX = 'share/voyage/browser/'
MAX_FILES = 1800
MAX_BYTES = 64 * 1024 * 1024


def stage(source: Path, destination: Path) -> dict:
    source = source.absolute()
    package = source / 'package.json'
    if not package.exists():
        return {}
    required = ['worker.mjs', 'guardian.py', 'package.json', 'package-lock.json',
                'node_modules/playwright-core/package.json']
    for name in required:
        if not (source / name).is_file():
            raise ValueError('Browser distribution incomplete: ' + name)
    dependencies = json.loads(package.read_text())['dependencies']
    installed = json.loads((source / required[-1]).read_text())['version']
    if dependencies.get('playwright-core') != installed:
        raise ValueError('Browser dependency does not match pinned version')
    paths = list(source.glob('*.mjs')) + list(source.glob('*.py')) + [package, source / 'package-lock.json']
    paths += list((source / 'node_modules' / 'playwright-core').rglob('*'))
    inventory, total = {}, 0
    for path in sorted(paths):
        info = path.lstat()
        if stat.S_ISDIR(info.st_mode):
            continue
        if not stat.S_ISREG(info.st_mode):
            raise ValueError('Nonregular browser distribution member')
        for parent in (path, *path.parents):
            if parent.is_symlink():
                raise ValueError('Symlink in browser distribution')
            if parent == source:
                break
        total += info.st_size
        if len(inventory) >= MAX_FILES or total > MAX_BYTES:
            raise ValueError('Browser distribution exceeds bounds')
        name = PREFIX + path.relative_to(source).as_posix()
        data = path.read_bytes()
        if len(data) != info.st_size:
            raise ValueError('Browser distribution changed during staging')
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.exists():
            raise ValueError('Browser destination already exists')
        target.write_bytes(data)
        target.chmod(0o644)
        inventory[name] = {'sha256': hashlib.sha256(data).hexdigest()}
    return inventory
