#!/bin/sh
# Bootstrap a complete, pinned Voyage release, then run its installer.
set -eu
fail() { printf 'Voyage installer: %s\n' "$*" >&2; exit 1; }
case "$(uname -s)/$(uname -m)" in
    Linux/x86_64) target=x86_64-unknown-linux-gnu ;;
    Linux/aarch64|Linux/arm64) target=aarch64-unknown-linux-gnu ;;
    *) fail 'installation currently supports Linux with a systemd user manager only.' ;;
esac
wizard=1
local_source=0
dev=0
rollback=0
source_free=0
expect_bin=0
for argument in "$@"; do
    if [ "$expect_bin" -eq 1 ]; then
        expect_bin=0
        continue
    fi
    case "$argument" in
        install|upgrade) wizard=0 ;;
        rollback) wizard=0; rollback=1 ;;
        --bin-dir) local_source=1; expect_bin=1 ;;
        --help|-h|--version|status|service-status|service-stop|service-uninstall) wizard=0; source_free=1 ;;
        install-user-service) wizard=0; source_free=1 ;;
        --dev) dev=1 ;;
    esac
done
[ "$dev/$local_source" != 1/1 ] || fail "--dev conflicts with --bin-dir."
if [ "$wizard" -eq 1 ] && ! ( : </dev/tty >/dev/tty ) 2>/dev/null; then
    fail 'the wizard needs an interactive terminal; use install --dry-run or install --start for explicit command-line installation.'
fi
launch() {
    if [ "$wizard" -eq 1 ]; then
        "$@" </dev/tty >/dev/tty 2>/dev/tty
    else
        "$@"
    fi
}
launch_local() {
    selected_installer=$1
    selected_bin=$2
    shift 2
    if [ "$rollback/$dev/$local_source/$source_free" = 0/0/0/0 ]; then
        launch "$selected_installer" --bin-dir "$selected_bin" "$@"
    else
        launch "$selected_installer" "$@"
    fi
}
if [ -n "${VOYAGE_RELEASE_DIR:-}" ]; then
    [ "$dev" -eq 0 ] || fail "VOYAGE_RELEASE_DIR conflicts with --dev."
    case "$VOYAGE_RELEASE_DIR" in /*) release_dir=$VOYAGE_RELEASE_DIR ;; *) release_dir="$PWD/$VOYAGE_RELEASE_DIR" ;; esac
    [ ! -d "$release_dir/bin" ] || release_dir="$release_dir/bin"
    launch_local "$release_dir/voyage-installer" "$release_dir" "$@"
    exit "$?"
fi
if [ -n "${VOYAGE_INSTALLER_BIN:-}" ]; then
    case "$VOYAGE_INSTALLER_BIN" in /*) installer=$VOYAGE_INSTALLER_BIN ;; *) installer="$PWD/$VOYAGE_INSTALLER_BIN" ;; esac
    [ -f "$installer" ] && [ -x "$installer" ] || fail 'VOYAGE_INSTALLER_BIN must name an executable file.'
    launch_local "$installer" "$(dirname -- "$installer")" "$@"
    exit "$?"
fi
for utility in curl python3; do
    command -v "$utility" >/dev/null 2>&1 || fail "required utility missing: $utility"
done
fetch() {
    curl --disable --proto '=https' --proto-redir '=https' --tlsv1.2 --fail --location --silent --show-error \
        --connect-timeout 10 --max-time 300 --max-filesize "${3:-536870912}" --output "$2" "$1"
}
umask 077
# The install engine rejects writable/symlinked ancestors. Never extract under /tmp.
tmp=$(python3 - <<'PY'
import os, pathlib, stat, tempfile
home = pathlib.Path.home()
cache = home / '.cache' / 'voyage'
for path in reversed((cache, *cache.parents)):
    if not path.exists():
        path.mkdir(mode=0o700)
    st = path.lstat()
    if not stat.S_ISDIR(st.st_mode) or st.st_uid not in (0, os.getuid()) or st.st_mode & 0o022:
        raise SystemExit(f'Unsafe bootstrap directory: {path}')
print(tempfile.mkdtemp(prefix='install-', dir=cache))
PY
) || fail 'cannot create a safe bootstrap directory.'
trap 'rm -rf "$tmp"' 0
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
version=${VOYAGE_VERSION:-${VOYAGE_INSTALLER_VERSION:-latest}}
if [ "$version" = latest ]; then
    fetch 'https://api.github.com/repos/o-psi/voyage/releases/latest' "$tmp/latest.json" 1048576 || fail 'no published release is available; set VOYAGE_RELEASE_DIR to an extracted full release or local build.'
    version=$(python3 - "$tmp/latest.json" <<'PY'
import json, sys
with open(sys.argv[1]) as stream:
    print(json.load(stream)['tag_name'])
PY
    ) || fail 'invalid latest-release response.'
fi
case "$version" in ''|.|..|*[!A-Za-z0-9._-]*) fail 'invalid VOYAGE_VERSION.' ;; esac
asset="voyage-$version-$target.tar.gz"
base="https://github.com/o-psi/voyage/releases/download/$version"
printf 'Downloading Voyage %s (%s)…\n' "$version" "$target" >&2
fetch "$base/$asset.sha256" "$tmp/checksum" 4096 || fail 'release checksum unavailable; check the published version and platform.'
fetch "$base/$asset" "$tmp/$asset" || fail 'full release download failed.'
python3 - "$tmp" "$asset" "$version" "$target" <<'PY'
import hashlib, json, pathlib, re, shutil, sys, tarfile
base, asset = pathlib.Path(sys.argv[1]), sys.argv[2]
manifest = (base / 'checksum').read_text().strip()
match = re.fullmatch(r'([0-9a-fA-F]{64})  ' + re.escape(asset), manifest)
if not match:
    raise SystemExit('Invalid release checksum manifest')
with (base / asset).open('rb') as stream:
    actual = hashlib.file_digest(stream, 'sha256').hexdigest()
if actual != match[1].lower():
    raise SystemExit('Release checksum mismatch')
root = asset.removesuffix('.tar.gz')
seen, size = set(), 0
with tarfile.open(base / asset, 'r:gz') as archive:
    members = []
    for member in archive:
        members.append(member)
        if len(members) > 4096:
            raise SystemExit('Release has too many entries')
        path = pathlib.PurePosixPath(member.name)
        if (not path.parts or path.parts[0] != root or path.is_absolute()
                or '..' in path.parts or str(path) in seen or member.size < 0
                or any(ord(c) < 32 or ord(c) == 127 for c in member.name)
                or not (member.isdir() or member.isfile())):
            raise SystemExit('Unsafe release archive entry')
        seen.add(str(path))
        size += member.size
        if size > 1073741824:
            raise SystemExit('Release exceeds extraction limit')
    for member in members:
        output = base / member.name
        if member.isdir():
            output.mkdir(parents=True, exist_ok=True, mode=0o700)
        else:
            output.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            with archive.extractfile(member) as source, output.open('xb') as destination:
                shutil.copyfileobj(source, destination)
            output.chmod(0o700 if output.parent == base / root / 'bin' else 0o600)
metadata = base / root / 'release.json'
if not metadata.is_file() or metadata.stat().st_size > 65536:
    raise SystemExit('Release manifest missing or oversized')
release = json.loads(metadata.read_text())
names = ('helm', 'vessel', 'voyage', 'voyage-installer')
if (release.get('schema_version') != 1 or release.get('version') != sys.argv[3]
        or release.get('target') != sys.argv[4] or set(release.get('binaries', {})) != set(names)):
    raise SystemExit('Release manifest identity/platform mismatch')
for name in names:
    binary = base / root / 'bin' / name
    if not binary.is_file():
        raise SystemExit(f'Release is missing {name}')
    with binary.open('rb') as stream:
        actual = hashlib.file_digest(stream, 'sha256').hexdigest()
    if release['binaries'][name].get('sha256') != actual:
        raise SystemExit(f'Release binary checksum mismatch: {name}')
PY
launch_local "$tmp/${asset%.tar.gz}/bin/voyage-installer" "$tmp/${asset%.tar.gz}/bin" "$@"
