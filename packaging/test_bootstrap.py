"""Offline bootstrap checks: fake installer/systemctl; no downloads or host writes."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'install.sh'


class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.log = self.root / 'args'
        self.executable('id', '#!/bin/sh\necho 1000\n')
        self.executable('systemctl', '#!/bin/sh\nexit "${SERVICE_EXIT:-0}"\n')
        self.executable('voyage-installer', '#!/bin/sh\nprintf "%s\\n" "$@" > "$ARGS_LOG"\nexit "${INSTALLER_EXIT:-0}"\n')
        self.env = {**os.environ, 'PATH': f'{self.bin}:' + os.environ['PATH'],
                    'VOYAGE_RELEASE_DIR': str(self.bin), 'ARGS_LOG': str(self.log)}
        self.env.pop('VOYAGE_INSTALLER_BIN', None)

    def executable(self, name, text):
        path = self.bin / name
        path.write_text(text)
        path.chmod(0o700)

    def run_script(self, *args):
        return subprocess.run(['sh', str(SCRIPT), *args], env=self.env,
                              stdin=subprocess.DEVNULL, capture_output=True,
                              text=True, timeout=15, start_new_session=True)

    def test_syntax(self):
        subprocess.run(['sh', '-n', str(SCRIPT)], check=True)

    def test_default(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_text().splitlines(),
                         ['--bin-dir', str(self.bin), 'install', '--start'])
        self.assertIn('export PATH="$HOME/.local/bin:$PATH"', result.stdout)
        self.assertIn('cd /path/to/your/project && helm', result.stdout)

    def test_explicit_no_start(self):
        result = self.run_script('install', '--no-start')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn('--start', self.log.read_text().splitlines())

    def test_dry_run_has_no_success_guidance(self):
        result = self.run_script('install', '--dry-run')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn('Next steps', result.stdout)

    def test_failure_has_no_success_guidance(self):
        self.env['INSTALLER_EXIT'] = '42'
        result = self.run_script()
        self.assertEqual(result.returncode, 42)
        self.assertNotIn('Next steps', result.stdout)

    def test_service_preflight(self):
        self.env['SERVICE_EXIT'] = '1'
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('no reachable systemd user manager', result.stderr)
        self.assertFalse(self.log.exists())

    def test_root_refused(self):
        self.executable('id', '#!/bin/sh\necho 0\n')
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('not root or sudo', result.stderr)
        self.assertFalse(self.log.exists())

    def test_old_python_refused(self):
        self.executable('python3', '#!/bin/sh\nexit 1\n')
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Python 3.11 or newer', result.stderr)
        self.assertFalse(self.log.exists())

    def test_status_bypasses_preflight_and_guidance(self):
        self.env['SERVICE_EXIT'] = '1'
        result = self.run_script('status')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_text().splitlines(), ['status'])
        self.assertNotIn('Next steps', result.stdout)

    def test_rollback_does_not_receive_source(self):
        result = self.run_script('rollback', '--no-start')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_text().splitlines(), ['rollback', '--no-start'])

    def test_installer_override(self):
        self.env.pop('VOYAGE_RELEASE_DIR')
        self.env['VOYAGE_INSTALLER_BIN'] = str(self.bin / 'voyage-installer')
        self.assertEqual(self.run_script().returncode, 0)
        self.assertIn('install\n--start', self.log.read_text())

    def test_download_glibc_refused_before_fetch(self):
        self.env.pop('VOYAGE_RELEASE_DIR')
        self.executable('curl', '#!/bin/sh\nexit 99\n')
        # Execute the actual embedded check with an injected libc observation.
        self.executable('python3', '#!' + sys.executable + '\n'
                        'import sys, os\n'
                        'if sys.argv[1] == "-c": exec(sys.argv[2])\n'
                        'else:\n'
                        '    os.confstr = lambda key: "glibc 2.38"\n'
                        '    exec(sys.stdin.read())\n')
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('glibc 2.39 or newer', result.stderr)
        self.assertNotIn('Downloading', result.stderr)

    def test_download_arm_refused_before_fetch(self):
        self.env.pop('VOYAGE_RELEASE_DIR')
        self.executable('uname', '#!/bin/sh\ncase "$1" in -s) echo Linux;; *) echo aarch64;; esac\n')
        self.executable('python3', '#!' + sys.executable + '\n'
                        'import sys, os\n'
                        'if sys.argv[1] == "-c": exec(sys.argv[2])\n'
                        'else:\n'
                        '    os.confstr = lambda key: "glibc 2.39"\n'
                        '    exec(sys.stdin.read())\n')
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('x86_64 only', result.stderr)
        self.assertNotIn('Downloading', result.stderr)

    def test_flags_only_still_need_wizard_tty(self):
        result = self.run_script('--no-start')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('wizard needs an interactive terminal', result.stderr)


if __name__ == '__main__':
    unittest.main()
