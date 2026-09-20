#!/usr/bin/env python3
"""Linux-only browser process owner: guardian.py NODE WORKER TMPDIR.

Arguments/environment are trusted parent configuration, never protocol input.
stdin/stdout are opaque bounded pipes; stderr is discarded, not payload-logged.
TMPDIR must be a fresh private directory. After *verified* descendant termination
and reaping, remove TMPDIR and write HOME/guardian-cleanup.json atomically.
Never remove worker.lock or interpret/reconcile action receipts.
The marker proves resource cleanup only, not external-action reconciliation.
A killed guardian or missing/false marker is an unresolved resource obligation.
"""
import ctypes
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import time

LIMIT = 4 * 1024 * 1024  # Per direction; overflow fails closed, never blocks EOF.
TERM_SECONDS = 1.0
KILL_SECONDS = 6.0


def identity(pid):
    try:
        fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        return (pid, int(fields[1]), fields[19])  # pid, ppid, starttime
    except (FileNotFoundError, ProcessLookupError):
        return None


def same(a, b):
    return a is not None and b is not None and (a[0], a[2]) == (b[0], b[2])


class Owner:
    def __init__(self):
        self.me = identity(os.getpid())
        self.known = {}
        self.reaped = 0
        self.child = None
        self.forced = False

    def scan(self):
        processes = {}
        for name in os.listdir('/proc'):
            if name.isdecimal():
                p = identity(int(name))
                if p:
                    processes[p[0]] = p
        owned = {self.me[0]: self.me}
        owned.update({pid: p for pid, p in processes.items()
                      if same(p, self.known.get(pid))})
        changed = True
        while changed:
            changed = False
            for pid, p in processes.items():
                parent = owned.get(p[1])
                if (pid not in owned and parent and int(parent[2]) <= int(p[2])
                        and same(parent, identity(parent[0]))):
                    owned[pid] = p
                    changed = True
        owned.pop(self.me[0], None)
        self.known.update(owned)
        return list(owned.values())

    @staticmethod
    def send(p, sig):
        # A pidfd binds the signal even if the PID is recycled after validation.
        try:
            fd = os.pidfd_open(p[0])
        except ProcessLookupError:
            return
        try:
            if same(p, identity(p[0])):
                signal.pidfd_send_signal(fd, sig)
        except ProcessLookupError:
            pass
        finally:
            os.close(fd)

    def reap(self):
        while True:
            try:
                pid, status = os.waitpid(-1, os.WNOHANG)
            except ChildProcessError:
                return True
            if not pid:
                return False
            self.reaped += 1
            if self.child and pid == self.child.pid:
                self.child.returncode = os.waitstatus_to_exitcode(status)

    def cleanup(self):
        start = time.monotonic()
        terminated = set()
        while True:
            self.reap()
            owned = self.scan()
            if not owned and self.reap():
                # ECHILD and no known survivors: no process remains to fork.
                return True
            elapsed = time.monotonic() - start
            for p in owned:
                key = (p[0], p[2])
                if elapsed >= TERM_SECONDS:
                    self.forced = True
                    self.send(p, signal.SIGKILL)
                elif key not in terminated:
                    self.send(p, signal.SIGTERM)
                    terminated.add(key)
            if elapsed >= TERM_SECONDS + KILL_SECONDS:
                return False
            time.sleep(0.025)


def private_directory(path):
    s = path.lstat()
    if (not stat.S_ISDIR(s.st_mode) or s.st_uid != os.getuid()
            or s.st_mode & 0o077 or path.resolve() != path
            or path == Path('/') or len(path.parts) < 3):
        raise ValueError('unsafe temporary directory')
    return (s.st_dev, s.st_ino)


def marker(tmp, original, home, home_identity, complete, reason, owner):
    if private_directory(home) != home_identity:
        raise ValueError('home identity changed')
    if private_directory(tmp) != original:
        raise ValueError('temporary directory identity changed')
    if complete:
        shutil.rmtree(tmp)
    status = owner.child.returncode if owner.child else None
    data = {'version': 1, 'observed': complete,
            'worker_status': status if status is not None else -255,
            'forced': owner.forced,
            'cleanup_complete': complete,
            'descendants_terminated': complete, 'descendants_reaped': complete,
            'temporary_cleaned': complete, 'reason': reason,
            'external_actions_reconciled': False,
            'observed_processes': len(owner.known), 'reaped_processes': owner.reaped}
    # No process IDs, paths, commands, URLs, messages or browser payloads.
    dest = home / '.guardian-cleanup.json.new'
    fd = os.open(dest, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as out:
        json.dump(data, out)
        out.flush()
        os.fsync(out.fileno())
    os.replace(dest, home / 'guardian-cleanup.json')
    fd = os.open(home, os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def relay(child, owner, stopping):
    # All four endpoints are nonblocking. Always observe parent stdin, including
    # when Node is stopped or stdout is backpressured. Bounded overflow terminates
    # ownership rather than dropping protocol bytes or waiting indefinitely.
    inputs, outputs = bytearray(), bytearray()
    for fd in (0, 1, child.stdin.fileno(), child.stdout.fileno()):
        os.set_blocking(fd, False)
    while not stopping[0]:
        owner.scan()
        if child.poll() is not None:
            return 'worker_exit', outputs
        with selectors.DefaultSelector() as sel:
            sel.register(0, selectors.EVENT_READ, 'parent')
            sel.register(child.stdout, selectors.EVENT_READ, 'worker')
            if inputs:
                sel.register(child.stdin, selectors.EVENT_WRITE, 'input')
            if outputs:
                sel.register(1, selectors.EVENT_WRITE, 'output')
            for key, _ in sel.select(0.05):
                try:
                    if key.data in ('parent', 'worker'):
                        data = os.read(key.fd, 65536)
                        if not data:
                            return ('parent_eof' if key.data == 'parent' else 'worker_eof'), outputs
                        buf = inputs if key.data == 'parent' else outputs
                        buf.extend(data)
                        if len(buf) > LIMIT:
                            return 'pipe_limit', outputs
                    else:
                        buf = inputs if key.data == 'input' else outputs
                        count = os.write(key.fd, buf)
                        del buf[:count]
                except BlockingIOError:
                    pass
                except (BrokenPipeError, ConnectionResetError):
                    return 'pipe_closed', outputs
    return 'signal', outputs


def flush(child, pending):
    # Preserve an already-written shutdown reply without making cleanup depend
    # on the parent's reader. Never wait for a descendant to close a pipe.
    os.set_blocking(child.stdout.fileno(), False)
    os.set_blocking(1, False)
    deadline = time.monotonic() + 0.25
    while time.monotonic() < deadline:
        try:
            data = os.read(child.stdout.fileno(), 65536)
            if data:
                pending.extend(data)
            if len(pending) > LIMIT:
                return
            if pending:
                del pending[:os.write(1, pending)]
            elif not data:
                return
        except BlockingIOError:
            time.sleep(0.005)
        except (BrokenPipeError, ConnectionResetError):
            return


def main():
    if (sys.platform != 'linux' or len(sys.argv) != 4
            or not hasattr(os, 'pidfd_open') or not hasattr(signal, 'pidfd_send_signal')):
        return 2
    node, worker, directory = sys.argv[1:]
    if not Path(node).is_absolute() or not Path(worker).is_absolute():
        return 2
    tmp = Path(directory)
    original = private_directory(tmp)
    home = Path(os.environ['HOME'])
    home_identity = private_directory(home)
    if home == tmp or tmp in home.parents or home in tmp.parents:
        return 2
    for name in ('guardian-cleanup.json', '.guardian-cleanup.json.new'):
        try:
            (home / name).unlink()
        except FileNotFoundError:
            pass
    # Fresh scratch prevents stale success markers being mistaken for this run.
    if any(tmp.iterdir()):
        return 2
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(36, 1, 0, 0, 0) != 0:  # PR_SET_CHILD_SUBREAPER
        return 2
    fd = os.pidfd_open(os.getpid())  # Fail before launch on unsupported kernels.
    os.close(fd)
    stopping = [False]
    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(sig, lambda *_: stopping.__setitem__(0, True))
    owner = Owner()
    reason, pending, complete = 'guardian_error', bytearray(), False
    try:
        child = subprocess.Popen([node, worker], stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                 env={**os.environ, 'TMPDIR': str(tmp)}, close_fds=True)
        owner.child = child
        reason, pending = relay(child, owner, stopping)
    finally:
        if owner.child:
            owner.child.stdin.close()
        complete = owner.cleanup()
        marker(tmp, original, home, home_identity, complete, reason, owner)
        if owner.child:
            flush(owner.child, pending)
            owner.child.stdout.close()
    return 0 if complete else 1


if __name__ == '__main__':
    try:
        sys.exit(main())
    except Exception:
        # Static diagnostic only: never publish private paths or child payloads.
        os.write(2, b'browser guardian failed; cleanup requires reconciliation\n')
        sys.exit(1)
