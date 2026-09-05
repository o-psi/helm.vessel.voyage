#!/bin/sh
# Downloads the standalone Voyage setup preview; setup actions are mocked.
set -eu
fail() { printf 'Voyage installer: %s\n' "$*" >&2; exit 1; }
if ! ( : </dev/tty >/dev/tty ) 2>/dev/null; then
    fail 'an interactive terminal is required; run this command in a terminal.'
fi
if [ -n "${VOYAGE_INSTALLER_BIN:-}" ]; then
    case "$VOYAGE_INSTALLER_BIN" in /*) ;; *) VOYAGE_INSTALLER_BIN="$PWD/$VOYAGE_INSTALLER_BIN" ;; esac
    [ -f "$VOYAGE_INSTALLER_BIN" ] && [ -x "$VOYAGE_INSTALLER_BIN" ] || fail 'VOYAGE_INSTALLER_BIN must name an executable file.'
    exec "$VOYAGE_INSTALLER_BIN" </dev/tty >/dev/tty 2>/dev/tty
fi
case "$(uname -s)/$(uname -m)" in
    Linux/x86_64) target=x86_64-unknown-linux-gnu ;;
    Darwin/x86_64) target=x86_64-apple-darwin ;;
    Darwin/arm64|Darwin/aarch64) target=aarch64-apple-darwin ;;
    *) fail 'unsupported platform; use a locally built voyage-installer with VOYAGE_INSTALLER_BIN.' ;;
esac
for utility in curl gzip mktemp chmod; do
    command -v "$utility" >/dev/null 2>&1 || fail "required utility missing: $utility"
done
if command -v sha256sum >/dev/null 2>&1; then
    hash_tool=sha256sum
elif command -v shasum >/dev/null 2>&1; then
    hash_tool=shasum
else
    fail 'SHA-256 verification requires sha256sum or shasum.'
fi
version=${VOYAGE_INSTALLER_VERSION:-latest}
case "$version" in ''|*[!A-Za-z0-9._-]*) fail 'invalid VOYAGE_INSTALLER_VERSION.' ;; esac
base=https://github.com/o-psi/voyage/releases
if [ "$version" = latest ]; then base="$base/latest/download"; else base="$base/download/$version"; fi
asset="voyage-installer-$target.gz"
umask 077
tmp=$(mktemp -d "${TMPDIR:-/tmp}/voyage-installer.XXXXXXXX") || fail 'cannot create temporary directory.'
trap 'rm -rf "$tmp"' 0
trap 'exit 130' INT
trap 'exit 143' TERM
fetch() {
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 --fail --location --silent --show-error \
        --connect-timeout 10 --max-time 120 --output "$2" "$1"
}
printf 'Downloading Voyage setup preview (%s)…\n' "$target" >&2
fetch "$base/$asset.sha256" "$tmp/checksum" || fail 'checksum download failed; installer assets may not be published yet. Use VOYAGE_INSTALLER_BIN for a local preview.'
fetch "$base/$asset" "$tmp/$asset" || fail 'installer download failed.'
manifest=$(cat "$tmp/checksum")
digest=${manifest%% *}
case "$digest" in ''|*[!0-9a-fA-F]*) fail 'invalid checksum manifest.' ;; esac
[ "${#digest}" -eq 64 ] && [ "$manifest" = "$digest  $asset" ] || fail 'invalid checksum manifest.'
if [ "$hash_tool" = sha256sum ]; then
    actual=$(sha256sum "$tmp/$asset")
else
    actual=$(shasum -a 256 "$tmp/$asset")
fi
actual=${actual%% *}
[ "$actual" = "$digest" ] || fail 'installer checksum mismatch.'
gzip -dc "$tmp/$asset" > "$tmp/voyage-installer" || fail 'installer decompression failed.'
chmod 700 "$tmp/voyage-installer"
"$tmp/voyage-installer" </dev/tty >/dev/tty 2>/dev/tty
