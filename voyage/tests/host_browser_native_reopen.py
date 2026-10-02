"""One source-owned TUI reopen, private fixture A only; never a command broker.

The parent already owns the live TUI/PTY and its original public credential route.
No credential, cookie, grant, command, endpoint or client path is accepted here.
"""
import hashlib
import json
import os
from pathlib import Path
import stat
import time
import uuid

LIMIT = 8192

def decode(raw):
    def pairs(values):
        result = {}
        for key, value in values:
            assert key not in result
            result[key] = value
        return result
    return json.loads(raw, object_pairs_hook=pairs, parse_constant=lambda _: (_ for _ in ()).throw(ValueError("invalid number")))


def encode(value):
    raw = json.dumps(value, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode()
    assert len(raw) <= LIMIT
    return raw


def inode(meta):
    return (meta.st_dev, meta.st_ino, meta.st_size, meta.st_mtime_ns, meta.st_ctime_ns)


def read_private(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(fd, 'rb') as source:
        meta = os.fstat(source.fileno())
        assert stat.S_ISREG(meta.st_mode) and meta.st_uid == os.getuid() and meta.st_nlink == 1 and not meta.st_mode & 0o077 and meta.st_size <= LIMIT
        raw = source.read(LIMIT + 1)
        assert len(raw) <= LIMIT and inode(meta) == inode(os.fstat(source.fileno())) == inode(Path(path).lstat())
    return raw, inode(meta)


def exclusive(path, value):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    with os.fdopen(fd, 'wb') as output:
        output.write(encode(value));output.flush();os.fsync(output.fileno())
    directory = os.open(Path(path).parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try: os.fsync(directory)
    finally: os.close(directory)


def process_identity(pid):
    proc = Path('/proc')/str(pid)
    fields = (proc/'stat').read_text().rsplit(')', 1)[1].split()
    assert fields[0] not in ('Z', 'T', 't', 'X', 'x') and proc.stat().st_uid == os.getuid()
    return int(fields[19]), os.readlink(proc/'exe')


class Reopen:
    def __init__(self, directory, session, label, title, root_client, program, client, capture, owner_root,
                 screen, paste, send, wait):
        self.directory = Path(directory)
        self.fd = os.open(self.directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        meta = os.fstat(self.fd)
        assert self.directory.is_absolute() and self.directory.resolve() == self.directory
        assert meta.st_uid == os.getuid() and not meta.st_mode & 0o077
        self.pin = (meta.st_dev, meta.st_ino)
        self.session, self.label, self.root_client, self.program = session, label, dict(root_client), dict(program)
        assert isinstance(title, str) and title.startswith('qualification-333-') and len(title) <= 64 and '\n' not in title
        self.title = title  # Trusted parent catalogue metadata, never child input.
        self.client, self.capture, self.owner_root = client, Path(capture), Path(owner_root)
        self.screen, self.paste, self.send, self.wait = screen, paste, send, wait
        self.attempted = False

    def close(self): os.close(self.fd)

    def verify(self):
        named, held = self.directory.lstat(), os.fstat(self.fd)
        assert stat.S_ISDIR(named.st_mode) and (named.st_dev, named.st_ino) == self.pin == (held.st_dev, held.st_ino)
        assert named.st_uid == os.getuid() and not named.st_mode & 0o077
        assert self.client['process'].poll() is None and self.client['pid'] == self.root_client['pid']
        start, executable = process_identity(self.root_client['pid'])
        assert start == self.root_client['start_ticks'] and executable == self.program['path']
        path = Path(executable)
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
        with os.fdopen(fd, 'rb') as source:
            meta = os.fstat(source.fileno());assert stat.S_ISREG(meta.st_mode) and 0 < meta.st_size <= 256*1024*1024
            digest = hashlib.sha256();total = 0
            while chunk := source.read(1024*1024):
                total += len(chunk);assert total <= meta.st_size;digest.update(chunk)
            assert total == meta.st_size and inode(meta) == inode(os.fstat(source.fileno())) == inode(path.lstat())
            assert digest.hexdigest() == self.program['sha256']
        assert process_identity(self.root_client['pid']) == (start, executable)

    def request(self, value):
        assert isinstance(value, dict) and set(value) == {'schema','id','action','session_id','label','client','expires_at_ms','digest'}
        assert type(value['schema']) is int and value['schema'] == 1 and value['action'] == 'native_reopen'
        assert str(uuid.UUID(value['id'])) == value['id']
        assert value['session_id'] == self.session and value['label'] == self.label
        assert value['client'] == self.root_client
        assert type(value['expires_at_ms']) is int and 0 < value['expires_at_ms'] - int(time.time()*1000) <= 45000
        digest = hashlib.sha256(encode({k:v for k,v in value.items() if k != 'digest'})).hexdigest()
        assert value['digest'] == digest
        return value

    def launcher(self):
        raw, _ = read_private(self.capture)
        path = Path(raw.decode())
        assert path.is_absolute() and path.resolve() == path and path.is_relative_to(self.owner_root) and path.name == 'open.html'
        meta = path.lstat()
        assert stat.S_ISREG(meta.st_mode) and meta.st_uid == os.getuid() and meta.st_nlink == 1 and not meta.st_mode & 0o077 and meta.st_size <= 65536
        return str(path)

    def panel(self):
        frame = self.screen(self.client)
        # panels::display removes the Markdown heading prefix in the real TUI.
        return self.title in frame and 'Host browser' in frame and ('Voyage: '+self.session) in frame

    def conversation(self):
        frame = self.screen(self.client)
        return self.title in frame and 'Host browser' not in frame and ('Voyage: '+self.session) not in frame

    def poll(self):
        request_path = self.directory/'request.json'
        if not request_path.exists() or self.attempted: return
        raw, request_pin = read_private(request_path)
        value = self.request(decode(raw))
        deadline = time.monotonic() + (value['expires_at_ms'] - int(time.time()*1000))/1000
        def remaining():
            seconds = min(deadline-time.monotonic(), (value['expires_at_ms']-int(time.time()*1000))/1000)
            assert seconds > 0, 'fixed reopen expiry reached'
            return seconds
        def observe(predicate):
            self.wait(predicate, min(15, remaining()))
            remaining()
        self.verify()
        old = self.launcher()
        # Durable exclusive pending receipt precedes every possible key effect.
        exclusive(self.directory/'pending.json', {'schema':1,'id':value['id'],'digest':value['digest'],'state':'pending','client':self.root_client,'session_id':self.session,'label':self.label,'program':self.program})
        self.attempted = True
        response = {'schema':1,'id':value['id'],'digest':value['digest'],'status':'unknown','outcome_unknown':True}
        try:
            assert read_private(request_path)[1] == request_pin
            assert self.panel()
            self.verify()
            # F6 left the Host browser panel open. It consumes Paste and Enter;
            # dismiss it once and observe the current frame before fixed input.
            remaining()
            self.send(self.client, '\x1b')
            observe(self.conversation)
            self.verify()
            assert read_private(request_path)[1] == request_pin and self.conversation()
            remaining()
            self.paste(self.client, '/browser detach');self.send(self.client, '\r')
            observe(lambda:self.conversation() and 'Viewer detached; host browser remains owned by Voyage' in self.screen(self.client))
            self.verify()
            assert read_private(request_path)[1] == request_pin and self.conversation()
            remaining()
            self.send(self.client, '\x1b[17~')  # Actual F6; same TUI and credential.
            def fresh():
                self.verify()
                if self.client['process'].poll() is not None: return False
                try: return self.launcher() != old
                except (OSError, AssertionError, UnicodeError): return False
            observe(fresh)
            new = self.launcher();assert new != old
            observe(self.panel)
            assert read_private(request_path)[1] == request_pin
            self.verify()
            remaining()
            response.update(status='observed', outcome_unknown=False, launcher=new, client=self.root_client)
        finally:
            # This action is never retried after an unknown keyboard outcome.
            exclusive(self.directory/'response.json', response)
