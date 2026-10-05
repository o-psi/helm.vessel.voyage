"""Embedded Linux acquisition worker. Never publishes binaries or starts services."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import time

MODE, ROOT = sys.argv[1], Path(sys.argv[2])
BINARIES = ('helm', 'vessel', 'voyage', 'voyage-installer')
os.umask(0o077)


class Failure(Exception):
    pass


def run(argv, label, timeout=300, cwd=ROOT, capture=False, output_limit=64 * 1024 * 1024):
    # The Rust owner bounds the whole worker process group and kills/reaps it on
    # cancellation or failure, including descendants left by a failed command.
    path = ROOT / 'command.log'
    with path.open('wb') as log:
        try:
            process = subprocess.Popen(argv, cwd=cwd, stdin=subprocess.DEVNULL,
                                       stdout=log, stderr=subprocess.STDOUT)
        except OSError:
            raise Failure(f'{label}: required executable unavailable ({argv[0]}).') from None
        deadline = time.monotonic() + timeout
        while process.poll() is None:
            if time.monotonic() >= deadline:
                raise Failure(f'{label} timed out; installation was not changed.')
            if path.stat().st_size > output_limit:
                raise Failure(f'{label} exceeded the diagnostic limit.')
            time.sleep(.05)
        if path.stat().st_size > output_limit:
            raise Failure(f'{label} exceeded the diagnostic limit.')
        if process.returncode:
            raise Failure(f'{label} failed (exit {process.returncode}). Check network access and required utilities; installation was not changed.')
    if capture:
        if path.stat().st_size > 65536:
            raise Failure(f'{label} returned oversized metadata.')
        return path.read_text().strip()


def fetch(url, path, label, limit=536870912):
    run(['curl', '--disable', '--proto', '=https', '--proto-redir', '=https',
         '--tlsv1.2', '--fail', '--location', '--silent', '--show-error',
         '--connect-timeout', '10', '--max-time', '300', '--max-filesize', str(limit),
         '--output', str(path), url], label, timeout=310)
    if path.stat().st_size > limit:
        raise Failure(f'{label} exceeded its download limit.')


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def extract(archive_path, expected):
    seen, size, members = set(), 0, []
    with tarfile.open(archive_path, 'r:gz') as archive:
        for member in archive:
            members.append(member)
            path = PurePosixPath(member.name)
            if (len(members) > 4096 or not path.parts or path.parts[0] != expected
                    or path.is_absolute() or '..' in path.parts
                    or str(path) in seen or '\\' in member.name
                    or any(ord(c) < 32 or ord(c) == 127 for c in member.name)
                    or not (member.isdir() or member.isfile()) or member.size < 0):
                raise Failure('Unsafe or oversized release archive.')
            seen.add(str(path))
            size += member.size
            if size > 1073741824:
                raise Failure('Release exceeds the 1 GiB extraction limit.')
        for member in members:
            output = ROOT / member.name
            if member.isdir():
                output.mkdir(parents=True, exist_ok=True, mode=0o700)
            else:
                output.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                with archive.extractfile(member) as source, output.open('xb') as destination:
                    shutil.copyfileobj(source, destination)
                output.chmod(0o700 if output.parent == ROOT / expected / 'bin' else 0o600)
    return ROOT / expected / 'bin'


def github_login():
    if not shutil.which('gh'):
        return False
    try:
        run(['gh', 'auth', 'status', '--hostname', 'github.com'],
            'Check existing GitHub CLI login', timeout=30)
        return True
    except Failure:
        return False


def github_api(endpoint, destination, label, limit, binary=False):
    argv = ['gh', 'api', '--hostname', 'github.com', endpoint]
    if binary:
        argv += ['--header', 'Accept: application/octet-stream']
    run(argv, label, timeout=310, output_limit=limit)
    shutil.copyfile(ROOT / 'command.log', destination)


def latest(target):
    metadata = ROOT / 'latest.json'
    try:
        if AUTHENTICATED:
            github_api('repos/o-psi/helm.vessel.voyage/releases/latest', metadata,
                       'Resolve latest published release', 1048576)
        else:
            fetch('https://api.github.com/repos/o-psi/helm.vessel.voyage/releases/latest', metadata,
                  'Resolve latest published release', 1048576)
    except Failure:
        raise Failure('Latest published stable release unavailable. Check GitHub/network availability (private repositories require gh auth login); use upgrade --dev for a public nightly or --bin-dir for an explicit local release. No fallback was installed.') from None
    data = json.loads(metadata.read_text())
    tag = data.get('tag_name', '')
    if (not isinstance(tag, str) or len(tag) > 128
            or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]*', tag)
            or data.get('draft') is not False or data.get('prerelease') is not False):
        raise Failure('Invalid latest stable release metadata.')
    name = f'voyage-{tag}-{target}'
    asset = name + '.tar.gz'
    base = f'https://github.com/o-psi/helm.vessel.voyage/releases/download/{tag}'
    def asset_fetch(name, destination, label, limit):
        if not AUTHENTICATED:
            return fetch(f'{base}/{name}', destination, label, limit)
        matches = [item for item in data.get('assets', []) if item.get('name') == name]
        if (len(matches) != 1 or type(matches[0].get('id')) is not int
                or matches[0]['id'] <= 0 or type(matches[0].get('size')) is not int
                or not 0 < matches[0]['size'] <= limit):
            raise Failure(f'Published release asset unavailable or oversized: {name}')
        github_api(f"repos/o-psi/helm.vessel.voyage/releases/assets/{matches[0]['id']}",
                   destination, label, limit, binary=True)

    checksum = ROOT / 'checksum'
    asset_fetch(asset + '.sha256', checksum, 'Download release checksum', 4096)
    match = re.fullmatch(r'([0-9a-fA-F]{64})  ' + re.escape(asset), checksum.read_text().strip())
    if not match:
        raise Failure('Invalid release checksum file.')
    archive = ROOT / asset
    asset_fetch(asset, archive, 'Download full release', 536870912)
    if digest(archive) != match[1].lower():
        raise Failure('Release archive checksum mismatch; nothing installed.')
    binaries = extract(archive, name)
    manifest_path = binaries.parent / 'release.json'
    if not manifest_path.is_file() or manifest_path.stat().st_size > 65536:
        raise Failure('Published release lacks a bounded release.json manifest.')
    manifest = json.loads(manifest_path.read_text())
    if manifest.get('version') != tag or manifest.get('target') != target:
        raise Failure('Release manifest does not match the pinned tag/platform.')
    return binaries, f'Published release {tag} ({target}); archive SHA-256 {match[1].lower()}'


def nightly(target):
    if target != 'x86_64-unknown-linux-gnu':
        raise Failure('Nightly downloads currently support Linux x86-64 only.')
    metadata = ROOT / 'releases.json'
    # Always public HTTPS, even when gh is installed or an ambient login expired.
    fetch('https://api.github.com/repos/o-psi/helm.vessel.voyage/releases?per_page=100',
          metadata, 'Find public nightly releases', 4194304)
    releases = json.loads(metadata.read_text())
    candidates = []
    for release in releases:
        match = re.fullmatch(r'nightly-([0-9]+\.[0-9]+\.[0-9]+-nightly\.[0-9]{8}\.[0-9]+\.[0-9]+)',
                            str(release.get('tag_name', '')))
        if match and release.get('prerelease') is True and release.get('draft') is False:
            candidates.append((tuple(map(int, re.findall(r'[0-9]+', match[1]))), release, match[1]))
    if not candidates:
        raise Failure('No public nightly is available. Wait for nightly publication; no fallback was installed.')
    _, chosen, version = max(candidates, key=lambda item: item[0])
    return download_nightly(chosen, version, target)


def pinned_nightly(target, version):
    if (not isinstance(version, str) or len(version) > 128
            or not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+-nightly\.[0-9]{8}\.[0-9]+\.[0-9]+', version)):
        raise Failure('Invalid pinned public nightly version.')
    if target != 'x86_64-unknown-linux-gnu':
        raise Failure('Nightly downloads currently support Linux x86-64 only.')
    tag = 'nightly-' + version
    metadata = ROOT / 'pinned-nightly.json'
    # Resolve this exact canonical tag, including older archives absent from the list.
    # No credentials, latest selection or metadata-supplied URL is consulted.
    fetch(f'https://api.github.com/repos/o-psi/helm.vessel.voyage/releases/tags/{tag}',
          metadata, 'Resolve exact public nightly', 1048576)
    chosen = json.loads(metadata.read_text())
    if (not isinstance(chosen, dict) or chosen.get('tag_name') != tag
            or chosen.get('prerelease') is not True or chosen.get('draft') is not False):
        raise Failure('Pinned public nightly release metadata mismatch.')
    return download_nightly(chosen, version, target)


def download_nightly(chosen, version, target):
    commit = chosen.get('target_commitish', '')
    if not re.fullmatch(r'[0-9a-f]{40}', str(commit)):
        raise Failure('Nightly release lacks an exact source commit.')
    name = f'voyage-{version}-{target}'
    archive_name = name + '.tar.gz'
    for asset, limit in ((archive_name, 536870912), (archive_name + '.sha256', 4096)):
        matches = [a for a in chosen.get('assets', []) if a.get('name') == asset]
        if (len(matches) != 1 or type(matches[0].get('size')) is not int
                or not 0 < matches[0]['size'] <= limit):
            raise Failure(f'Public nightly asset missing or oversized: {asset}')
        # Construct the repository-owned URL; never execute/download a metadata-supplied URL.
        fetch(f'https://github.com/o-psi/helm.vessel.voyage/releases/download/{chosen["tag_name"]}/{asset}',
              ROOT / asset, 'Download pinned public nightly', limit)
    checksum = (ROOT / (archive_name + '.sha256')).read_text().strip()
    match = re.fullmatch(r'([0-9a-f]{64})  ' + re.escape(archive_name), checksum)
    if not match or digest(ROOT / archive_name) != match[1]:
        raise Failure('Nightly archive checksum mismatch.')
    binaries = extract(ROOT / archive_name, name)
    manifest_path, build_path = binaries.parent / 'release.json', binaries.parent / 'BUILD.txt'
    if (not manifest_path.is_file() or manifest_path.stat().st_size > 65536
            or not build_path.is_file() or build_path.stat().st_size > 4096):
        raise Failure('Nightly source or release manifest missing or oversized.')
    manifest = json.loads(manifest_path.read_text())
    if (manifest.get('schema_version') != 1 or manifest.get('version') != version
            or manifest.get('target') != target or f'Source: {commit}\n' not in build_path.read_text()
            or not manifest.get('assets')):
        raise Failure('Nightly source, version, platform or browser inventory mismatch.')
    return binaries, f'Nightly {version}; source {commit}; archive SHA-256 {match[1]}'


def execute():
    if sys.version_info < (3, 11):
        raise Failure('Automatic upgrades require Python 3.11 or newer.')
    arch = {'x86_64': 'x86_64', 'aarch64': 'aarch64', 'arm64': 'aarch64'}.get(platform.machine())
    if platform.system() != 'Linux' or not arch:
        raise Failure('Automatic upgrades support Linux x86_64/aarch64 only.')
    global AUTHENTICATED
    os.environ['GH_PROMPT_DISABLED'] = '1'
    AUTHENTICATED = MODE == 'latest' and (len(sys.argv) < 4 or sys.argv[3] != 'public') and github_login()
    target = f'{arch}-unknown-linux-gnu'
    if MODE not in ('latest', 'nightly', 'nightly-pinned'):
        raise Failure('Unsupported acquisition mode.')
    if MODE == 'nightly-pinned':
        if len(sys.argv) != 5:
            raise Failure('Pinned public nightly version is required.')
        binaries, description = pinned_nightly(target, sys.argv[4])
    else:
        binaries, description = {'latest': latest, 'nightly': nightly}[MODE](target)
    for name in BINARIES:
        if not (binaries / name).is_file():
            raise Failure(f'Release is missing {name}.')
    (ROOT / 'prepared.json').write_text(json.dumps(dict(bin_dir=str(binaries), description=description)))


try:
    execute()
except Failure as error:
    (ROOT / 'error.txt').write_text(str(error))
    sys.exit(1)
except Exception:
    (ROOT / 'error.txt').write_text('Invalid or unreadable upgrade source; no installation changes applied.')
    sys.exit(1)
