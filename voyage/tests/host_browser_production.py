"""Actual public TLS browser qualification launcher; existing owned fixture Voyages only.

Requires a private operator config and normally authenticated Web storage state.
Never creates a grant/Voyage/provider, rewrites TLS, or accesses an ambient browser.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import tempfile
import re
import time
import sys
import uuid

# With -I, admit only this maintained source directory for sibling fixtures.
# No user-site/PYTHONPATH startup or caller-provided import directory is used.
sys.path.insert(0, str(Path(__file__).resolve().parent))

from ui_journeys import launch, paste, screen, send, pty_helpers
from host_browser import wait
from host_browser_native_reopen import Reopen, exclusive, process_identity


def private_json(path, limit=1024*1024, with_identity=False):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(fd, 'rb') as source:
        meta = os.fstat(source.fileno())
        assert stat.S_ISREG(meta.st_mode) and meta.st_uid == os.getuid()
        assert meta.st_nlink == 1 and not meta.st_mode & 0o077 and meta.st_size <= limit
        data = source.read(limit+1); after = os.fstat(source.fileno())
        assert len(data) <= limit and (meta.st_dev, meta.st_ino, meta.st_size, meta.st_mtime_ns, meta.st_ctime_ns) == \
               (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns)
        assert after.st_uid == os.getuid() and after.st_nlink == 1 and not after.st_mode & 0o077
        named = Path(path).lstat()
        assert (named.st_dev, named.st_ino) == (meta.st_dev, meta.st_ino)
    value = json.loads(data)
    return (value, (meta.st_dev, meta.st_ino)) if with_identity else value


def native_route_id(config):
    """Exact existing LegacyRoute identity, without reading a credential."""
    route = config['native_route']
    directory = os.fsencode(route['directory']) if route['mode'] == 'local' else b''
    access = None if route['mode'] == 'local' else os.fsencode(config['access_file'])
    digest = hashlib.sha256(b'helm-legacy-route-v1' + len(directory).to_bytes(8, 'little') + directory +
                            (b'\0' if access is None else b'\1' + access)).digest()
    identity = bytearray(digest[:16])
    identity[6] = (identity[6] & 0x0f) | 0x80
    identity[8] = (identity[8] & 0x3f) | 0x80
    return str(uuid.UUID(bytes=bytes(identity)))


def fixture_observation(catalogue, snapshot, item):
    matches = [entry for entry in catalogue if entry['session_id'] == item['id']]
    assert len(matches) == 1, 'fixture absent or ambiguous in authenticated catalogue'
    process = matches[0]
    assert process.get('name') == item['title'] and process['workspace'] == item['workspace']
    assert str(uuid.UUID(process['incarnation'])) == process['incarnation'] and uuid.UUID(process['incarnation']).int
    assert not process.get('archive') and not process.get('deletion')
    assert snapshot['session_id'] == item['id'] and snapshot.get('name') == item['title']
    assert snapshot['messages'] == [] and not snapshot.get('run') and snapshot.get('pending_cleanup_run') is None
    assert type(snapshot['revision']) is int and snapshot['revision'] > 0
    return {'session_id':item['id'], 'incarnation':process['incarnation'], 'revision':snapshot['revision']}


def composer_ready(frame):
    # These footer labels exist only for a selected live catalogue view with no
    # input-owning panel/terminal/review. The empty composer alone also appears
    # before catalogue hydration, so it cannot prove readiness by itself.
    return 'Ask anything, or describe what you want to do...' in frame and any(
        hint in '\n'.join(frame.splitlines()[-3:]) for hint in (
            'Enter Submit new turn', 'Waiting for voyage · draft retained'))


def settings_selection(frame, session, route_id):
    selected = re.search(r'Voyage\s+([0-9a-f-]{36})\s+·\s+Vessel connection\s+([0-9a-f-]{36})', frame)
    revision = re.search(r'Revision\s+([0-9]+)\s+·\s+access:', frame)
    if not selected or not revision or 'Effective settings · read-only observations' not in frame:
        return None
    assert selected.groups() == (session, route_id), 'actual TUI selected a different voyage or route'
    assert 'Source: last authenticated executing-host snapshot (not a new fetch).' in frame
    assert 'STALE:' not in frame and 'Authenticated settings snapshot unavailable.' not in frame
    return int(revision.group(1))


def bootstrap_native(client, item, config, metrics, capture, output, observe_fixture, discovery_check,
                     screen_fn=screen, paste_fn=paste, send_fn=send, wait_fn=wait):
    """One normal TUI selection/F6 after authenticated socket and metadata proof.

    No initial fixture-title assumption, no retry of any keyboard action, and no
    browser/transport substitute. Private durable intent precedes the first key.
    """
    route = config['native_route']; route_id = native_route_id(config)
    deadline = time.monotonic() + 40
    # launch() first execs the maintained PTY shim. Pin the final Helm image only
    # once its own authenticated meter exists, without mistaking that shim for it.
    start, executable = None, config['native_helm_program']['path']
    pin = None
    def meter():
        nonlocal pin, start
        assert client['process'].poll() is None
        actual_start, actual_executable = process_identity(client['pid'])
        assert actual_executable == executable
        if start is None: start = actual_start
        assert actual_start == start
        # The source-owned writer updates this same inode every 200ms. Repeat only
        # bounded metadata reads across a partial write; never replay a key/effect.
        read_deadline = min(deadline, time.monotonic()+1)
        while True:
            try:
                value, identity = private_json(metrics, 4096, with_identity=True)
                break
            except (json.JSONDecodeError, AssertionError):
                assert time.monotonic() < read_deadline, 'native meter read unavailable'
                time.sleep(.025)
        if pin is None: pin = identity
        assert identity == pin, 'native meter inode changed'
        assert value['status'] == 'observed' and value['pid'] == client['pid'] and value['start_ticks'] == start
        assert value['label'] == 'native-' + item['label'] and value['routes'] == 1 and value['active'] == 1
        assert value['connections'] == 1 and value['attempts'] == 1 and value['disconnected'] == 0
        assert value['handshake_failures'] == 0 and value['transport_failures'] == 0
        for key in ('socket_id','vessel_id'):
            assert str(uuid.UUID(value[key])) == value[key] and uuid.UUID(value[key]).int
        assert value['transport'] == ('local_ws' if route['mode']=='local' else 'public_wss')
        assert 0 <= time.time()*1000-value['captured_at_ms'] <= 1000
        if route['mode']=='local':
            assert value['vessel_id'] == route['expected_vessel_id'] and value['authority'] == 'existing local account owner'
        discovery_check()
        return value
    def initial_meter():
        if not Path(metrics).exists(): return False
        try: return meter()
        except (json.JSONDecodeError, AssertionError): return False
    initial = wait_fn(initial_meter, 5)
    assert initial, 'authenticated native meter unavailable'
    def current():
        value = meter()
        assert (value['socket_id'],value['vessel_id']) == (initial['socket_id'],initial['vessel_id']), 'native socket changed during bootstrap'
        return value
    def observe(predicate, seconds=40):
        def checked():
            current()
            return predicate()
        remaining = deadline-time.monotonic()
        assert remaining > 0, 'native bootstrap deadline exceeded'
        value = wait_fn(checked, min(seconds, remaining))
        assert time.monotonic() < deadline, 'native bootstrap deadline exceeded'
        return value
    observe(lambda:composer_ready(screen_fn(client)))
    intent = {'schema':1,'session_id':item['id'],'label':item['label'],'client':{'pid':client['pid'],'start_ticks':start},
              'route_id':route_id,'vessel_id':initial['vessel_id'],'socket_id':initial['socket_id'],
              'meter_inode':list(pin),'state':'pending','actions':['use_exact_session','settings_read','dismiss_settings','f6']}
    exclusive(output/('native-'+item['label']+'-bootstrap-intent.json'), intent)
    result = {**intent,'state':'unknown','f6_dispatched':False}
    try:
        current();paste_fn(client,'/use '+item['id']);send_fn(client,'\r')
        observe(lambda:composer_ready(screen_fn(client)))
        current();paste_fn(client,'/settings');send_fn(client,'\r')
        selected_revision = observe(lambda:settings_selection(screen_fn(client),item['id'],route_id))
        # Fresh normal catalogue/snapshot queries retain their own authenticated
        # route. The exact TUI route/SID/revision must match before F6. Runtime
        # ownership checks in Helm still bind viewer preparation to that owner.
        selected = observe_fixture()
        current()
        assert selected_revision == selected['revision'], 'TUI snapshot revision differs from current fixture metadata'
        assert settings_selection(screen_fn(client),item['id'],route_id) == selected['revision']
        result['selected_owner'] = selected
        exclusive(output/('native-'+item['label']+'-bootstrap-selected.json'),
                  {**intent,'state':'selected_observed','selected_owner':selected})
        send_fn(client,'\x1b')
        observe(lambda:composer_ready(screen_fn(client)) and
                'Effective settings · read-only observations' not in screen_fn(client))
        current();result['f6_dispatched'] = True;send_fn(client,'\x1b[17~')
        observe(lambda:Path(capture).exists())
        observe(lambda:'Host browser' in screen_fn(client) and ('Voyage: '+item['id']) in screen_fn(client),20)
        result['state'] = 'observed'
        return {'pid':client['pid'],'start_ticks':start,'label':'native-'+item['label'],'descendants':True}
    finally:
        # Refusal/unknown is retained once; never type /use, settings or F6 again.
        exclusive(output/('native-'+item['label']+'-bootstrap-result.json'), result)


def native_selection(config):
    route = config.get('native_route', {'mode':'public'})
    assert isinstance(route, dict) and route.get('mode') in ('local','public')
    native_args = ['--no-start']
    discovery_pin = None
    if route['mode'] == 'local':
        assert set(route) == {'mode','directory','expected_vessel_id'} and 'access_file' not in config
        assert str(uuid.UUID(route['expected_vessel_id'])) == route['expected_vessel_id'] and uuid.UUID(route['expected_vessel_id']).int
        directory = Path(route['directory'])
        assert directory.is_absolute() and directory.resolve(strict=True) == directory and str(directory) != '/'
        meta = directory.lstat()
        assert stat.S_ISDIR(meta.st_mode) and meta.st_uid == os.getuid() and not meta.st_mode & 0o077
        # Shape/inode only; the token is consumed solely by ordinary Helm discovery.
        fd = os.open(directory/'process-http.json',os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK|os.O_CLOEXEC)
        try:
            meta = os.fstat(fd)
            assert stat.S_ISREG(meta.st_mode) and meta.st_uid == os.getuid() and meta.st_nlink == 1 and not meta.st_mode & 0o077 and 0 < meta.st_size < 4096
            discovery_pin = (meta.st_dev,meta.st_ino,meta.st_size,meta.st_mtime_ns,meta.st_ctime_ns)
        finally:os.close(fd)
        native_args += ['--directory',str(directory)]
    else:
        assert set(route) == {'mode'}
        access = Path(config['access_file']);assert access.is_absolute()
        fd = os.open(access,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK|os.O_CLOEXEC)
        try:
            meta = os.fstat(fd)
            assert stat.S_ISREG(meta.st_mode) and meta.st_uid == os.getuid() and meta.st_nlink == 1 and not meta.st_mode & 0o077
        finally:os.close(fd)
        native_args += ['--access-file',str(access)]
    return route, native_args, discovery_pin


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    args = parser.parse_args()
    assert __debug__ and sys.platform == 'linux' and os.getuid() > 0, 'ordinary Linux fixture user required'
    config = private_json(args.config)
    assert config['schema'] == 1 and len(config['sessions']) == 2
    assert len({item['id'] for item in config['sessions']}) == 2
    assert len({item['label'] for item in config['sessions']}) == 2
    for item in config['sessions']:
        assert str(uuid.UUID(item['id'])) == item['id']
        assert item['title'].startswith('qualification-333-')
        assert re.fullmatch(r'[A-Za-z0-9_.-]{1,40}', item['label']) and len(item['title']) <= 64
        assert Path(item['workspace']).is_absolute() and item['workspace'] != '/'
    helm, node = Path(config['helm']), Path(config['node'])
    assert helm.is_absolute() and helm.is_file() and node.is_absolute() and node.is_file()
    route, native_args, discovery_pin = native_selection(config)
    config['native_route'] = route
    if route['mode']=='local':directory=Path(route['directory'])
    os.umask(0o077)
    output = Path(config['output']).resolve(); output.mkdir(mode=0o700)
    root = Path(tempfile.mkdtemp(prefix='browser-production-'))
    env = {'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8', 'TERM': 'xterm-256color'}
    for key in ('HOME','XDG_CONFIG_HOME','XDG_STATE_HOME','XDG_DATA_HOME','XDG_CACHE_HOME'):
        folder=root/key.lower();folder.mkdir(mode=0o700);env[key]=str(folder)
    if 'VOYAGE_CREDENTIAL_KEY_FILE' in os.environ:
        env['VOYAGE_CREDENTIAL_KEY_FILE']=os.environ['VOYAGE_CREDENTIAL_KEY_FILE']
    opener=root/'bin';opener.mkdir(mode=0o700)
    (opener/'xdg-open').write_text('#!/usr/bin/python3\nimport os,pathlib,sys\np=pathlib.Path(sys.argv[1]).resolve()\nassert p.is_relative_to(pathlib.Path(os.environ["HOME"]).parent) and p.name=="open.html"\npathlib.Path(os.environ["QUALIFICATION_LAUNCHER"]).write_text(str(p))\n')
    (opener/'xdg-open').chmod(0o700)
    env['PATH']=str(opener)+':'+env['PATH']
    config['native_helm_program']={'path':str(helm.resolve()),'sha256':hashlib.sha256(helm.resolve().read_bytes()).hexdigest()}
    report={'schema':1,'status':'pending','cleanup':{},'helm_sha256':hashlib.sha256(helm.read_bytes()).hexdigest(),
            'source_sha256':{name:hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest() for name in
                             ('host_browser_production.py','host_browser_production.mjs','host_browser_cost.py',
                              'host_browser_client_cost.mjs','host_browser_cua_cost.mjs','host_browser_production_probe.py','host_browser_production_site.py','host_browser_native_reopen.py','host_browser_native_reopen.mjs','host_browser_qualification_windows.mjs')}}
    clients=[]
    reopen=None
    config['native_reopen_mailbox']=str(root/'native-reopen')
    Path(config['native_reopen_mailbox']).mkdir(mode=0o700)
    config['native_metrics_files']=[]
    config['client_roots']=[]
    driver=Path(__file__).with_name('host_browser_production.mjs')
    def cli(*parts):
        result=subprocess.run([str(helm),'connect',*native_args,*parts],
                              env=env,capture_output=True,timeout=30)
        assert result.returncode==0,'public route refused; private diagnostics remain with operator'
        assert len(result.stdout)<=4*1024*1024
        return json.loads(result.stdout)
    try:
        preflight_config=root/'preflight-config.json';preflight_config.write_text(json.dumps(config));preflight_config.chmod(0o600)
        with (output/'preflight-private.log').open('xb') as log:
            checked=subprocess.run([str(node),str(driver),'--validate',str(preflight_config)],stdout=log,stderr=log,timeout=10)
        assert checked.returncode==0, 'private qualification config refused before browser effects'
        catalogue=cli('list');assert isinstance(catalogue,list)
        if route['mode']=='local':
            meta=(directory/'process-http.json').lstat()
            assert discovery_pin==(meta.st_dev,meta.st_ino,meta.st_size,meta.st_mtime_ns,meta.st_ctime_ns)
            report['native_route']={'mode':'local','expected_vessel_id':route['expected_vessel_id'],'discovery_inode':discovery_pin[:2],'authority':'existing local account owner'}
        for item in config['sessions']:
            snap=cli('inspect',item['id'])
            fixture_observation(catalogue,snap,item)
            captured=root/(item['label']+'-launcher')
            metrics=output/('native-'+item['label']+'-'+route['mode']+'-transport-private.json')
            config['native_metrics_files'].append(str(metrics))
            meter_prefix='HELM_QUALIFICATION_LOCAL' if route['mode']=='local' else 'HELM_QUALIFICATION_WSS'
            use_env={**env,'QUALIFICATION_LAUNCHER':str(captured),
                     meter_prefix+'_OUTPUT':str(metrics),meter_prefix+'_LABEL':'native-'+item['label']}
            client=launch([str(helm),'connect',*native_args],use_env,root,output/(item['label']+'-private.pty'),120,36)
            clients.append(client)
            def discovery_check():
                if route['mode']=='local':
                    meta=(directory/'process-http.json').lstat()
                    assert discovery_pin==(meta.st_dev,meta.st_ino,meta.st_size,meta.st_mtime_ns,meta.st_ctime_ns)
            def observe_fixture():
                fresh=cli('inspect',item['id'])
                return fixture_observation(cli('list'),fresh,item)
            config['client_roots'].append(bootstrap_native(
                client,item,config,metrics,captured,output,observe_fixture,discovery_check))
            item['native_launcher']=captured.read_text()
        config['output']=str(output)
        first=config['sessions'][0]
        reopen=Reopen(config['native_reopen_mailbox'],first['id'],first['label'],first['title'],config['client_roots'][0],config['native_helm_program'],
                      clients[0],root/(first['label']+'-launcher'),root,screen,paste,send,wait)
        prepared=root/'private-driver.json';prepared.write_text(json.dumps(config));prepared.chmod(0o600)
        with (output/'driver-private.log').open('xb') as log:
            child=subprocess.Popen([str(node),str(driver),str(prepared)],stdout=log,stderr=log)
            deadline=time.monotonic()+600
            try:
                while child.poll() is None:
                    assert time.monotonic()<deadline, 'production driver deadline exceeded'
                    assert all(len(client['output'])<=32*1024*1024 for client in clients), 'owned PTY output exceeds bound'
                    assert log.tell()<=4*1024*1024, 'private driver diagnostic bound exceeded'
                    reopen.poll()
                    time.sleep(.2)
            finally:
                if child.poll() is None:
                    child.terminate()
                    try: child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        child.kill();child.wait(timeout=5)
        assert child.returncode==0,'production qualification incomplete/failed; inspect fixed report and private log'
        for item in config['sessions']:
            snap=cli('inspect',item['id'])
            assert not snap['messages'] and not snap.get('run') and snap.get('pending_cleanup_run') is None
        report['status']='passed'
    except BaseException:
        report['status']='failed_or_incomplete'
        raise
    finally:
        if reopen is not None:
            report['cleanup']['native_reopen_attempted']=reopen.attempted
            reopen.close()
        for client in clients:
            try:
                if client['process'].poll() is None:
                    send(client,'\x11')
                    try: client['process'].wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        report['cleanup']['native_client_forced']=True
                pty_helpers.stop_pty(client,wait)
                assert client['process'].returncode==0
            except Exception:
                report['cleanup']['native_client_unresolved']=True
                if client['process'].poll() is None:
                    client['process'].send_signal(signal.SIGTERM)
        report['cleanup']['native_clients_exited']=all(client['process'].poll() is not None for client in clients)
        (output/'launcher-report.json').write_text(json.dumps(report,indent=2))
    assert report['status']=='passed' and report['cleanup']['native_clients_exited'] and \
           not report['cleanup'].get('native_client_unresolved') and not report['cleanup'].get('native_client_forced')


if __name__=='__main__':
    if not __debug__:
        raise SystemExit('Qualification validation must be enabled.')
    main()
