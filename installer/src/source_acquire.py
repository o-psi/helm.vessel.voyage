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
            raise Failure(f'{label} failed (exit {process.returncode}). Check network access and required build utilities; installation was not changed.')
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
            github_api('repos/o-psi/voyage/releases/latest', metadata,
                       'Resolve latest published release', 1048576)
        else:
            fetch('https://api.github.com/repos/o-psi/voyage/releases/latest', metadata,
                  'Resolve latest published release', 1048576)
    except Failure:
        raise Failure('Latest published release unavailable. Check GitHub/network availability (private repositories require gh auth login); if no release is published, use upgrade --dev explicitly to build main, or --bin-dir for a local build. No fallback was installed.') from None
    data = json.loads(metadata.read_text())
    tag = data.get('tag_name', '')
    if (not isinstance(tag, str) or len(tag) > 128
            or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]*', tag)
            or data.get('draft') is not False or data.get('prerelease') is not False):
        raise Failure('Invalid latest stable release metadata.')
    name = f'voyage-{tag}-{target}'
    asset = name + '.tar.gz'
    base = f'https://github.com/o-psi/voyage/releases/download/{tag}'
    def asset_fetch(name, destination, label, limit):
        if not AUTHENTICATED:
            return fetch(f'{base}/{name}', destination, label, limit)
        matches = [item for item in data.get('assets', []) if item.get('name') == name]
        if (len(matches) != 1 or type(matches[0].get('id')) is not int
                or matches[0]['id'] <= 0 or type(matches[0].get('size')) is not int
                or not 0 < matches[0]['size'] <= limit):
            raise Failure(f'Published release asset unavailable or oversized: {name}')
        github_api(f"repos/o-psi/voyage/releases/assets/{matches[0]['id']}",
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


def main_build(target):
    # Explicit --dev trusts build code from canonical main, not local work or
    # global Git configuration. This is not an OS sandbox for Cargo build scripts.
    os.environ.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null',
                      GIT_TERMINAL_PROMPT='0', GIT_ASKPASS='/bin/false',
                      GIT_ALLOW_PROTOCOL='https', CARGO_TERM_COLOR='never',
                      CARGO_BUILD_JOBS='2', CARGO_NET_RETRY='2', CARGO_HTTP_TIMEOUT='60')
    cargo_home = ROOT / 'cargo-home'
    cargo_home.mkdir(mode=0o700)
    os.environ['CARGO_HOME'] = str(cargo_home)
    os.environ['RUSTUP_TOOLCHAIN'] = 'stable'
    checkout = ROOT / 'checkout'
    checkout.mkdir(mode=0o700)
    git = ['git', '-c', 'credential.helper=', '-c', 'core.hooksPath=/dev/null',
           '-c', 'http.followRedirects=false']
    if AUTHENTICATED:
        git += ['-c', 'credential.helper=!gh auth git-credential']
    run(git + ['init', '--quiet', str(checkout)], 'Create private development checkout')
    run(git + ['fetch', '--depth=1', 'https://github.com/o-psi/voyage.git', 'refs/heads/main'],
        'Fetch GitHub main', cwd=checkout)
    commit = run(git + ['rev-parse', '--verify', 'FETCH_HEAD^{commit}'],
                 'Resolve main commit', cwd=checkout, capture=True)
    if not re.fullmatch(r'[0-9a-f]{40}', commit):
        raise Failure('Invalid main commit identity.')
    run(git + ['checkout', '--detach', commit], 'Check out pinned main', cwd=checkout)
    run(['cargo', 'build', '--workspace', '--release', '--locked', '--jobs', '2',
         '--target', target, '--target-dir', str(ROOT / 'target')],
        'Build pinned main (requires stable Rust, native build tools and registry access)',
        cwd=checkout, timeout=3600)
    built = ROOT / 'target' / target / 'release'
    binaries = ROOT / 'development' / 'bin'
    binaries.mkdir(parents=True, mode=0o700)
    for name in BINARIES:
        source = built / name
        if not source.is_file() or source.is_symlink() or source.stat().st_size > 1073741824:
            raise Failure(f'Development build is missing a valid {name}.')
        shutil.copyfile(source, binaries / name)
        (binaries / name).chmod(0o700)
    manifest = dict(schema_version=1, version=f'dev-{commit}', target=target,
                    binaries={name: dict(sha256=digest(binaries / name)) for name in BINARIES})
    (binaries.parent / 'release.json').write_text(json.dumps(manifest))
    return binaries, f'Development build from GitHub main, pinned commit {commit} ({target})'


def execute():
    if sys.version_info < (3, 11):
        raise Failure('Automatic upgrades require Python 3.11 or newer.')
    arch = {'x86_64': 'x86_64', 'aarch64': 'aarch64', 'arm64': 'aarch64'}.get(platform.machine())
    if platform.system() != 'Linux' or not arch:
        raise Failure('Automatic upgrades support Linux x86_64/aarch64 only.')
    global AUTHENTICATED
    os.environ['GH_PROMPT_DISABLED'] = '1'
    AUTHENTICATED = github_login()
    target = f'{arch}-unknown-linux-gnu'
    binaries, description = latest(target) if MODE == 'latest' else main_build(target)
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
