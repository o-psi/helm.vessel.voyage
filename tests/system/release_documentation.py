#!/usr/bin/env python3
"""Verify full-archive documentation and no-clobber publication in disposable storage."""
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import zipfile
import sys
import tempfile
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()



def verify_published_archives(directory):
    documents = (ROOT / 'scripts/release-documents.txt').read_text().splitlines()
    archives = sorted([*directory.glob('voyage-*.tar.gz'), *directory.glob('voyage-*.zip')])
    assert archives, 'no full release archives found'
    for path in archives:
        windows = path.suffix == '.zip'
        opened = zipfile.ZipFile(path) if windows else tarfile.open(path)
        with opened as package_file:
            raw_names = package_file.namelist() if windows else package_file.getnames()
            mapping = {name.replace('\\', '/'): name for name in raw_names}
            prefixes = {name.split('/')[0] for name in mapping}
            assert len(prefixes) == 1, (path.name, prefixes)
            prefix = next(iter(prefixes))
            relative = {name.removeprefix(prefix + '/') for name in mapping}
            assert set(documents) <= relative, (path.name, sorted(set(documents) - relative))
            def read(name):
                raw = mapping[prefix + '/' + name]
                return package_file.read(raw) if windows else package_file.extractfile(raw).read()
            for name in documents:
                assert read(name) == (ROOT / name).read_bytes(), (path.name, name, 'document differs from source')
                if name.endswith('.md'):
                    for link in re.findall(r'\]\(([^)]+)\)', read(name).decode()):
                        parsed = urlsplit(link)
                        if parsed.scheme or not parsed.path:
                            continue
                        target = os.path.normpath(str(Path(name).parent / unquote(parsed.path))).replace('\\', '/')
                        assert target in relative, (path.name, name, link)
            suffix = '.exe' if windows else ''
            for binary in ['helm', 'vessel', 'voyage-installer']:
                assert f'bin/{binary}{suffix}' in relative, (path.name, binary)
        checksum = path.with_name(path.name + '.sha256')
        assert checksum.read_text().split()[0].lower() == digest(path), path.name
    print(f'{len(archives)} published archive layouts, guide links and checksums: passed')


def main():
    with tempfile.TemporaryDirectory(prefix='voyage-release-docs-') as temporary:
        root = Path(temporary)
        documents = (ROOT / 'scripts/release-documents.txt').read_text().splitlines()
        for name in [*documents, 'scripts/release-documents.txt', 'scripts/package-release']:
            destination = root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, destination)
        # Unlisted local material must not enter published archives.
        (root / 'docs/untracked-private-note.md').write_text('UNTRACKED_PACKAGING_CANARY')
        binaries = root / 'target/release'
        binaries.mkdir(parents=True)
        for name, variable in [('helm', 'HELM_BIN'), ('vessel', 'VESSEL_BIN'), ('voyage-installer', 'INSTALLER_BIN')]:
            source = Path(os.environ.get(variable, ROOT / 'target/release' / name)).resolve()
            shutil.copy2(source, binaries / name)
        environment = os.environ.copy()
        environment.pop('TARGET', None)
        environment['HOME'] = str(root / 'home')
        environment['XDG_CONFIG_HOME'] = str(root / 'home/config')
        environment['XDG_DATA_HOME'] = str(root / 'home/data')

        def package(label, success=True):
            result = subprocess.run(['bash', 'scripts/package-release', label], cwd=root, env=environment, capture_output=True, text=True, timeout=60)
            assert (result.returncode == 0) == success, (label, result.returncode, result.stdout, result.stderr)

        package('documentation-test')
        archive, = (root / 'dist').glob('*.tar.gz')
        checksum = archive.with_name(archive.name + '.sha256')
        assert checksum.read_text().split()[0] == digest(archive)
        with tarfile.open(archive) as package_file:
            names = package_file.getnames()
            prefix = names[0].split('/')[0]
            relative = {name.removeprefix(prefix + '/') for name in names}
            assert set(documents) <= relative, sorted(set(documents) - relative)
            assert 'docs/untracked-private-note.md' not in relative
            for name in documents:
                if not name.endswith('.md'):
                    continue
                text = package_file.extractfile(prefix + '/' + name).read().decode()
                for link in re.findall(r'\]\(([^)]+)\)', text):
                    parsed = urlsplit(link)
                    if parsed.scheme or not parsed.path:
                        continue
                    target = os.path.normpath(str(Path(name).parent / unquote(parsed.path)))
                    assert target in relative, (name, link, target)
            for name in ['bin/helm', 'bin/vessel', 'bin/voyage-installer', 'share/man/man1/helm.1']:
                assert name in relative, name
            extracted = root / 'extracted'
            package_file.extractall(extracted, filter='data')
        for binary in ['helm', 'vessel', 'voyage-installer']:
            result = subprocess.run([str(extracted / prefix / 'bin' / binary), '--version'], env=environment, capture_output=True, text=True, timeout=10)
            assert result.returncode == 0, binary
        verify_published_archives(root / 'dist')
        windows_archive = root / 'dist/voyage-documentation-windows.zip'
        with tarfile.open(archive) as source, zipfile.ZipFile(windows_archive, 'w') as output:
            for member in source.getmembers():
                if not member.isfile():
                    continue
                name = member.name
                if name in [prefix + '/bin/' + binary for binary in ['helm', 'vessel', 'voyage-installer']]:
                    name += '.exe'
                output.writestr(name, source.extractfile(member).read())
        windows_archive.with_name(windows_archive.name + '.sha256').write_text(digest(windows_archive) + '  ' + windows_archive.name + '\n')
        # This covers the common Windows layout verifier, not native PowerShell execution.
        verify_published_archives(root / 'dist')
        before = {p.name: digest(p) for p in (root / 'dist').iterdir() if p.is_file()}
        package('documentation-test', False)
        assert before == {p.name: digest(p) for p in (root / 'dist').iterdir() if p.is_file()}
        for invalid in ['../escape', '/absolute', 'bad label', '']:
            package(invalid, False)
        environment['TARGET'] = '../invalid-target'
        package('bad-target', False)
        environment.pop('TARGET')
        assert not list((root / 'dist').glob('*bad-target*'))
        guide = root / 'helm/config.example.toml'
        guide_bytes = guide.read_bytes()
        guide.unlink()
        guide.symlink_to(root / 'docs/untracked-private-note.md')
        package('symlink-guide', False)
        guide.unlink()
        guide.write_bytes(guide_bytes)
        assert not list((root / 'dist').glob('*symlink-guide*'))
        original_manifest = (root / 'scripts/release-documents.txt').read_text()
        (root / 'scripts/release-documents.txt').write_text('../outside.md\n')
        package('invalid-manifest', False)
        (root / 'scripts/release-documents.txt').write_text(original_manifest)
        assert not list((root / 'dist').glob('*invalid-manifest*'))
        missing = root / documents[-1]
        missing.unlink()
        package('missing-guide', False)
        assert not list((root / 'dist').glob('*missing-guide*'))
        assert before == {p.name: digest(p) for p in (root / 'dist').iterdir() if p.is_file()}
        assert not list((root / 'dist').glob('.package-*'))
        assert not list((root / 'dist').glob('*.lock'))
        print('release documentation, extracted binaries, links, and publication failures: passed')


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--archives':
        verify_published_archives(Path(sys.argv[2]))
    else:
        assert len(sys.argv) == 1, 'usage: release_documentation.py [--archives DIRECTORY]'
        main()
