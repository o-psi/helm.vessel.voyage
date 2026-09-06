#!/usr/bin/env python3
"""Real subprocess regressions for the shared local quality runner."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]


def command(source):
    return [sys.executable, '-c', source]


class Project:
    def __init__(self, root, gates):
        self.root = root
        (root / 'scripts').mkdir(parents=True)
        shutil.copy2(ROOT / 'scripts/check-quality', root / 'scripts/check-quality')
        (root / 'scripts/quality-gates.json').write_text(json.dumps(gates))
        (root / '.gitignore').write_text('/.quality-runs/\n/target/\n/dist/\n')
        (root / 'source.txt').write_text('original')
        self.env = dict(os.environ, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull,
                        GIT_AUTHOR_NAME='Quality Fixture', GIT_AUTHOR_EMAIL='quality@example.invalid',
                        GIT_COMMITTER_NAME='Quality Fixture', GIT_COMMITTER_EMAIL='quality@example.invalid')
        for key in ['CARGO_TARGET_DIR', 'CARGO_BUILD_TARGET', 'TARGET']:
            self.env.pop(key, None)
        self.git('init', '-q')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.root, env=self.env, text=True).strip()

    def run(self, *args, env=None):
        return subprocess.run([str(self.root / 'scripts/check-quality'), *args], cwd=self.root,
                              env=env or self.env, capture_output=True, text=True, timeout=30)

    def reports(self):
        return [json.loads(path.read_text()) for path in sorted((self.root / '.quality-runs').glob('*/results.json'))]


def gates_and_workflows():
    gates = json.loads((ROOT / 'scripts/quality-gates.json').read_text())
    required = json.loads((ROOT / 'tests/fixtures/quality-required-gates.json').read_text())
    assert len(required) == 65
    assert len({gate['id'] for gate in gates}) == len(gates)
    indexed = {gate['id']: gate for gate in gates}
    for gate in required:
        assert indexed[gate['id']] == gate, gate['id']
    release = next(n for n, gate in enumerate(gates) if gate['id'] == 'release-build')
    for n, gate in enumerate(gates):
        if gate['argv'][:1] == ['python3']:
            assert (ROOT / gate['argv'][1]).is_file()
        if gate['id'] in {item['id'] for item in required[4:60]}:
            assert n > release
    for host in ['.github', '.forgejo']:
        quality = (ROOT / host / 'workflows/ci.yml').read_text()
        assert quality.count('workflow_dispatch:') == 1
        assert '\n  push:' not in quality and 'pull_request:' not in quality
        assert quality.count('run: ./scripts/check-quality') == 1
        assert 'if: always()' in quality and 'path: .quality-runs/' in quality
        assert 'include-hidden-files: true' in quality
        assert 'package-release' not in quality and 'cargo test' not in quality
        release_file = ROOT / host / 'workflows/release.yml'
        expected = json.loads((ROOT / 'tests/fixtures/quality-release-hashes.json').read_text())
        assert hashlib.sha256(release_file.read_bytes()).hexdigest() == expected[host]


def pipelines(root):
    gates = [{'id': 'ok', 'argv': command('print("first gate")')},
             {'id': 'bad', 'argv': command('import sys; print("failure detail"); sys.exit(7)')},
             {'id': 'never', 'argv': command('raise AssertionError("must not run")')}]
    p = Project(root / 'failure', gates)
    result = p.run()
    assert result.returncode == 7, result.stderr
    report = p.reports()[0]
    assert report['status'] == 'failed' and report['exit_code'] == 7
    assert report['revision'] == p.git('rev-parse', 'HEAD') and report['tree'] == p.git('rev-parse', 'HEAD^{tree}')
    assert [gate['id'] for gate in report['gates']] == ['ok', 'bad']
    assert 'failure detail' in Path(report['gates'][1]['log']).read_text()
    assert all(gate['started_at'] and gate['finished_at'] and gate['command'] for gate in report['gates'])
    # Failure releases ownership, and a second run preserves the first evidence.
    assert p.run().returncode == 7
    assert len(p.reports()) == 2 and len({x['version'] for x in p.reports()}) == 2
    p = Project(root / 'dirty', [{'id': 'write', 'argv': command('from pathlib import Path; Path("source.txt").write_text("changed")')}])
    assert p.run().returncode != 0
    assert p.reports()[0]['status'] == 'invalidated'
    assert p.run().returncode != 0 and len(p.reports()) == 1
    p = Project(root / 'launch', [{'id': 'missing', 'argv': [str(root / 'does-not-exist')]}])
    assert p.run().returncode == 127 and p.reports()[0]['gates'][0]['status'] == 'launch_failed'


def ownership_and_timeout(root):
    source = 'from pathlib import Path; import time; Path("target").mkdir(exist_ok=True); Path("target/ready").touch(); time.sleep(30)'
    p = Project(root / 'ownership', [{'id': 'hold', 'argv': command(source)}])
    process = subprocess.Popen([str(p.root / 'scripts/check-quality')], cwd=p.root, env=p.env,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        deadline = time.monotonic() + 10
        while not (p.root / 'target/ready').exists():
            assert process.poll() is None and time.monotonic() < deadline
            time.sleep(.01)
        result = p.run()
        assert result.returncode != 0 and 'another full quality suite' in result.stderr
        linked = root / 'linked-owner'
        p.git('worktree', 'add', '--detach', str(linked))
        result = subprocess.run([str(linked / 'scripts/check-quality')], cwd=linked,
                                env=p.env, capture_output=True, text=True, timeout=10)
        assert result.returncode != 0 and 'another full quality suite' in result.stderr
        process.send_signal(signal.SIGINT)
        process.communicate(timeout=15)
        assert process.returncode == 130
        assert p.reports()[0]['status'] == 'interrupted'
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
    result = p.run('--timeout-seconds', '.1')
    assert result.returncode == 124, result.stderr
    assert any(x['gates'][0]['status'] == 'timeout' for x in p.reports())
    for name, delay in [('output-limit', 30), ('output-exit', 0)]:
        loud = Project(root / name, [{'id': 'loud', 'argv': command(
            'import os,time; block=b"x"*1048576; [os.write(1,block) for _ in range(65)]; time.sleep(' + str(delay) + ')')}])
        assert loud.run().returncode == 125
        assert loud.reports()[0]['gates'][0]['status'] == 'output_limit'
    env = dict(p.env, CARGO_TARGET_DIR=str(root / 'different-target'))
    assert p.run(env=env).returncode != 0
    env = dict(p.env, TARGET='other-target')
    assert p.run(env=env).returncode != 0


def binaries_and_checksums(root):
    source = '''import os
from pathlib import Path
for name,binary in [('HELM_BIN','helm'),('VESSEL_BIN','vessel'),('INSTALLER_BIN','voyage-installer')]:
 assert Path(os.environ[name]) == Path.cwd()/'target/release'/binary
assert os.environ['CARGO_BUILD_JOBS']=='1'
assert os.environ['RUSTUP_TOOLCHAIN']=='stable'
assert Path(os.environ['CARGO_TARGET_DIR']) == Path.cwd()/'target'
assert os.environ['PRE_CONTROL_HELM_BIN']=='fixture-old-helm'
'''
    p = Project(root / 'env', [{'id': 'env', 'argv': command(source)}])
    env = dict(p.env, HELM_BIN='/stale/helm', VESSEL_BIN='/stale/vessel', INSTALLER_BIN='/stale/installer', PRE_CONTROL_HELM_BIN='fixture-old-helm')
    assert p.run(env=env).returncode == 0
    assert p.reports()[0]['status'] == 'passed'
    assert p.reports()[0]['saved_peer_inputs']['PRE_CONTROL_HELM_BIN'] is True
    p = Project(root / 'checksum', [{'id': 'checksum', 'argv': ['sha256sum', '-c'], 'cwd': 'dist', 'glob_args': '*.sha256'}])
    (p.root / 'dist').mkdir()
    assert p.run().returncode != 0
    (p.root / 'dist/asset').write_bytes(b'original')
    digest = hashlib.sha256(b'original').hexdigest()
    (p.root / 'dist/asset.sha256').write_text(digest + '  asset\n')
    assert p.run().returncode == 0
    (p.root / 'dist/asset').write_bytes(b'corrupt')
    assert p.run().returncode != 0
    assert (p.root / 'dist/asset').read_bytes() == b'corrupt'


def process_alive(pid):
    try:
        return Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[0] not in ('Z', 'X')
    except FileNotFoundError:
        return False


def descendants(root):
    child = "import os,signal,time; from pathlib import Path; signal.signal(signal.SIGINT,signal.SIG_IGN); signal.signal(signal.SIGTERM,signal.SIG_IGN); Path('target/child').write_text(str(os.getpid())); time.sleep(30)"
    leader = "import subprocess,sys,time; from pathlib import Path; Path('target').mkdir(exist_ok=True); subprocess.Popen([sys.executable,'-c'," + repr(child) + "]); "
    for mode in ['normal', 'interrupt', 'hangup']:
        source = leader + ("time.sleep(.3)" if mode == 'normal' else "time.sleep(30)")
        p = Project(root / mode, [{'id': 'descendant', 'argv': command(source)}])
        process = subprocess.Popen([str(p.root / 'scripts/check-quality')], cwd=p.root, env=p.env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        pid = None
        try:
            deadline = time.monotonic() + 10
            while not (p.root / 'target/child').exists():
                assert process.poll() is None and time.monotonic() < deadline
                time.sleep(.01)
            pid = int((p.root / 'target/child').read_text())
            if mode != 'normal':
                process.send_signal(signal.SIGINT if mode == 'interrupt' else signal.SIGHUP)
            process.communicate(timeout=15)
            assert process.returncode == (125 if mode == 'normal' else 130)
            assert not process_alive(pid), 'quality released ownership with an active descendant'
        finally:
            if process.poll() is None:
                process.kill(); process.wait(timeout=5)
            if pid is not None and process_alive(pid):
                os.kill(pid, signal.SIGKILL)


def cargo_configuration(root):
    source = "import json,subprocess; from pathlib import Path; data=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'])); assert Path(data['target_directory'])==Path.cwd()/'target'"
    p = Project(root / 'cargo', [{'id': 'metadata', 'argv': command(source)}])
    (p.root / '.cargo').mkdir(); (p.root / 'src').mkdir()
    (p.root / '.cargo/config.toml').write_text('[build]\ntarget-dir="redirected-output"\n')
    (p.root / 'Cargo.toml').write_text('[package]\nname="quality-fixture"\nversion="0.0.0"\nedition="2024"\n')
    (p.root / 'src/lib.rs').write_text('')
    # Cargo metadata can create a lock file; record it before exact-tree checks.
    subprocess.run(['cargo','generate-lockfile'],cwd=p.root,env=p.env,check=True,capture_output=True)
    p.git('add','.'); p.git('commit','-qm','cargo fixture')
    result = p.run()
    assert result.returncode == 0, (result.stdout, result.stderr, p.reports())
    (p.root / '.cargo/config.toml').write_text('[build]\ntarget="nonexistent-target"\n')
    p.git('add', '.'); p.git('commit', '-qm', 'configured target')
    result = p.run()
    assert result.returncode != 0 and 'without build.target' in result.stderr
    # Ancestor and CARGO_HOME configuration use the same discovery guard.
    (p.root / '.cargo/config.toml').write_text('')
    p.git('add', '.'); p.git('commit', '-qm', 'native configuration')
    home = root / 'cargo-home'; home.mkdir()
    (home / 'config').write_text('[build]\ntarget=["nonexistent-target"]\n')
    result = p.run(env=dict(p.env, CARGO_HOME=str(home)))
    assert result.returncode != 0 and 'without build.target' in result.stderr
    ancestor = root / '.cargo'; ancestor.mkdir()
    (ancestor / 'config.toml').write_text('[build]\ntarget="nonexistent-target"\n')
    result = p.run()
    assert result.returncode != 0 and 'without build.target' in result.stderr


def main():
    gates_and_workflows()
    with tempfile.TemporaryDirectory(prefix='voyage-quality-runner-') as temporary:
        root = Path(temporary)
        pipelines(root)
        ownership_and_timeout(root)
        binaries_and_checksums(root)
        descendants(root)
        cargo_configuration(root)
    print('Quality runner: gate union, failure, revision, ownership, interruption, paths and checksums PASS')


if __name__ == '__main__':
    main()
