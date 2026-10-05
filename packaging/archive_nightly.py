"""Create immutable target-labelled nightly archives (packaging, not native validation)."""
import argparse
import hashlib
from pathlib import Path
import re
import shutil
import tarfile
import tempfile
import zipfile

import stage_nightly


def archive(binary_dir: Path, output: Path, version: str, source_sha: str,
            target: str, browser_source: Path) -> Path:
    if target not in stage_nightly.TARGETS:
        raise ValueError('Unsupported nightly archive target')
    if not stage_nightly.VERSION.fullmatch(version):
        raise ValueError('Expected unique nightly version')
    if not re.fullmatch(r'[0-9a-f]{40}', source_sha):
        raise ValueError('Expected exact source commit')
    suffix = stage_nightly.TARGETS[target]
    name = f'voyage-{version}-{target}'
    extension = '.zip' if suffix else '.tar.gz'
    output.mkdir(parents=True, exist_ok=True)
    destination = output / (name + extension)
    checksum = output / (name + extension + '.sha256')
    if destination.exists() or checksum.exists():
        raise ValueError('Refusing to overwrite nightly evidence')
    # Preparation completes privately before publishing either output file.
    with tempfile.TemporaryDirectory(prefix='.nightly-', dir=output) as temp:
        root = Path(temp) / name
        (root / 'bin').mkdir(parents=True)
        for binary in stage_nightly.BINARIES:
            original = binary_dir / (binary + suffix)
            if original.is_symlink() or not original.is_file() or not original.stat().st_size:
                raise ValueError(f'Invalid nightly binary: {original.name}')
            shutil.copyfile(original, root / 'bin' / original.name)
            (root / 'bin' / original.name).chmod(0o755)
        stage_nightly.stage(root, version, browser_source, target)
        (root / 'BUILD.txt').write_text(
            f'Development build: {version}\nSource: {source_sha}\nTarget: {target}\n'
            'No tests run by the build workflow. Not a stable release.\n'
            'Archive integrity does not establish native installation or service support.\n',
            encoding='utf-8')
        prepared = Path(temp) / destination.name
        if suffix:
            with zipfile.ZipFile(prepared, 'w', compression=zipfile.ZIP_DEFLATED) as bundle:
                for path in sorted(root.rglob('*')):
                    if path.is_file():
                        bundle.write(path, path.relative_to(Path(temp)).as_posix())
        else:
            with tarfile.open(prepared, 'w:gz') as bundle:
                bundle.add(root, arcname=name)
        with prepared.open('rb') as staged:
            digest = hashlib.file_digest(staged, 'sha256').hexdigest()
        # Exclusive creation also refuses an output that appeared during staging.
        with destination.open('xb') as final:
            with prepared.open('rb') as staged:
                shutil.copyfileobj(staged, final)
        with checksum.open('x', encoding='ascii') as final:
            final.write(f'{digest}  {destination.name}\n')
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--source-sha', required=True)
    parser.add_argument('--target', choices=tuple(stage_nightly.TARGETS), required=True)
    parser.add_argument('--browser-source', type=Path, default=Path('voyage/browser'))
    args = parser.parse_args()
    archive(args.bin_dir, args.output, args.version, args.source_sha, args.target, args.browser_source)


if __name__ == '__main__':
    main()
