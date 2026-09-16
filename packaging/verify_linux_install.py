#!/usr/bin/env python3
"""Real installer checks with a simulated inactive systemd manager in namespaces.

No host service access, credentials or installation writes. Requires Linux bwrap.
This does not establish real systemd activation/reboot persistence.
"""
import argparse
import os
from pathlib import Path
import subprocess

SYSTEMCTL = r'''#!/bin/sh
set -eu
printf '%s\n' "$*" >> /h/systemctl.calls
[ "$1" = --user ] && [ "$2" = --no-pager ] || exit 90
shift 2
case "$*" in
 daemon-reload) ;;
 'show --value --property UnitPath') echo /h/c/systemd/user ;;
 'show voyage-vessel.service --value --property ActiveState') echo inactive ;;
 'show voyage-vessel.service --value --property UnitFileState') echo disabled ;;
 'show voyage-vessel.service --value --property FragmentPath')
   if [ -f /h/c/systemd/user/voyage-vessel.service ]; then
     echo /h/c/systemd/user/voyage-vessel.service
   fi ;;
 'show voyage-vessel.service --value --property KillMode') echo process ;;
 'show voyage-vessel.service --value --property DropInPaths'|\
 'show voyage-vessel.service --value --property InvocationID'|\
 'show voyage-vessel.service --value --property ExecStop'|\
 'show voyage-vessel.service --value --property ExecStopPost') ;;
 *) echo "Unexpected systemctl request: $*" >&2; exit 92 ;;
esac
'''

CHECKS = r'''
/r/bin/voyage-installer install --bin-dir /r/bin --dry-run --no-start
test ! -e /h/.local/share/voyage/install
/r/bin/voyage-installer install --bin-dir /r/bin --no-start
before=$(readlink /h/.local/share/voyage/install/current)
/r/bin/voyage-installer upgrade --bin-dir /r/bin --no-start
test "$before" = "$(readlink /h/.local/share/voyage/install/current)"
/h/.local/bin/voyage-installer status
for binary in helm vessel voyage voyage-installer; do
  test "$(/h/.local/bin/$binary --version)" = "$binary $EXPECTED_VERSION"
  cmp /r/bin/$binary /h/.local/bin/$binary
done
grep '^KillMode=process$' /h/c/systemd/user/voyage-vessel.service
'''

HOSTED = r'''
# Pinned bootstrap installs into fresh private HOME using public HTTPS only.
VOYAGE_VERSION=v$EXPECTED_VERSION sh /bootstrap install --no-start
before=$(readlink /h/.local/share/voyage/install/current)
for binary in helm vessel voyage voyage-installer; do
  test "$(/h/.local/bin/$binary --version)" = "$binary $EXPECTED_VERSION"
  cmp /r/bin/$binary /h/.local/bin/$binary
done
# Unpinned bootstrap resolves latest, and installer upgrade performs its own
# independent latest-release acquisition. Both must preserve exact binary bytes.
sh /bootstrap install --no-start
/h/.local/bin/voyage-installer upgrade --no-start
test "$before" = "$(readlink /h/.local/share/voyage/install/current)"
for binary in helm vessel voyage voyage-installer; do
  cmp /r/bin/$binary /h/.local/bin/$binary
done
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--release-dir', required=True, type=Path)
    parser.add_argument('--version', default='1.0.0')
    parser.add_argument('--hosted', action='store_true', help='Download published pinned/latest assets; network required')
    args = parser.parse_args()
    root = args.release_dir.resolve(strict=True)
    for binary in ('helm', 'vessel', 'voyage', 'voyage-installer'):
        if not (root / 'bin' / binary).is_file():
            parser.error(f'missing release binary: {binary}')
    fd = os.memfd_create('systemctl-fixture', 0)
    try:
        os.write(fd, SYSTEMCTL.encode())
        os.lseek(fd, 0, 0)
        command = ['bwrap', '--unshare-user', '--unshare-pid', '--unshare-ipc', '--unshare-uts',
                   '--die-with-parent', '--new-session', '--ro-bind', '/usr', '/usr',
                   '--symlink', 'usr/bin', '/bin', '--symlink', 'usr/lib', '/lib',
                   '--symlink', 'usr/lib', '/lib64', '--proc', '/proc', '--dev', '/dev',
                   '--tmpfs', '/run', '--tmpfs', '/tmp', '--tmpfs', '/h', '--chmod', '0700', '/h',
                   '--ro-bind', str(root), '/r', '--perms', '0755', '--ro-bind-data', str(fd), '/usr/bin/systemctl']
        if args.hosted:
            command += ['--ro-bind', '/etc/ssl', '/etc/ssl', '--ro-bind', '/etc/resolv.conf', '/etc/resolv.conf',
                        '--ro-bind', str(Path(__file__).resolve().parent.parent / 'install.sh'), '/bootstrap']
        else:
            command += ['--unshare-net']
        command += ['--clearenv', '--setenv', 'PATH', '/usr/bin', '--setenv', 'HOME', '/h',
                    '--setenv', 'XDG_CONFIG_HOME', '/h/c', '--setenv', 'XDG_STATE_HOME', '/h/s',
                    '--setenv', 'XDG_RUNTIME_DIR', '/run', '--setenv', 'EXPECTED_VERSION', args.version,
                    '--chdir', '/h', '/usr/bin/sh', '-eu', '-c', HOSTED if args.hosted else CHECKS]
        subprocess.run(command, pass_fds=(fd,), timeout=600 if args.hosted else 180, check=True)
    finally:
        os.close(fd)
    print('PASS: actual binary installation with simulated inactive systemd; host services untouched')


if __name__ == '__main__':
    main()
