"""Quiescent ordinary schema-1 snapshot/proof. Never edit or downgrade live schema."""
import hashlib, json, os, pathlib, sqlite3, stat, sys
LIMIT = 128 * 1024 * 1024
BASE = ('voyages', 'incarnations', 'lifecycle_commands', 'creation_receipts', 'legacy_imports')
AUTHORITY = ('execution_identities', 'execution_bindings', 'administrator_grants',
             'administrator_revocations', 'administrator_authority', 'administrative_owners',
             'administrator_owner_commands', 'execution_reviews', 'execution_review_controls')

def checked(path, limit=LIMIT):
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
    fd = os.open(path, flags)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_nlink != 1 or info.st_mode & 0o077 or info.st_size > limit:
            raise ValueError('private ordinary state refused')
        with os.fdopen(fd, 'rb', closefd=False) as stream:
            data = stream.read(limit + 1)
        if len(data) > limit: raise ValueError('ordinary state exceeds bound')
        return data
    finally: os.close(fd)

def digest_file(path):
    return hashlib.sha256(checked(path)).hexdigest()

def database(path):
    checked(path, 512 * 1024 * 1024)
    if any(path.with_name(path.name+suffix).exists() for suffix in ('-wal','-shm')):
        raise ValueError('unexpected legacy WAL state')
    db = sqlite3.connect(path.as_uri() + '?mode=ro', uri=True, timeout=2)
    db.execute('PRAGMA query_only=ON')
    db.execute('BEGIN')
    if db.execute('PRAGMA quick_check').fetchone() != ('ok',):
        raise ValueError('ordinary catalogue integrity unavailable')
    return db

def canonical(db):
    sha = hashlib.sha256()
    for table in BASE:
        sha.update(table.encode())
        for row in db.execute('SELECT * FROM ' + table + ' ORDER BY 1,2'):
            encoded = [ {'bytes': value.hex()} if isinstance(value, bytes) else value for value in row]
            sha.update(json.dumps(encoded, sort_keys=True, separators=(',', ':')).encode())
    names = {row[0] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    for table in AUTHORITY:
        if table in names and db.execute('SELECT count(*) FROM '+table).fetchone()[0]:
            raise ValueError('new execution authority prevents legacy rollback')
    return sha.hexdigest()

def inventory(state, db):
    rows = db.execute('SELECT session_id,registration FROM voyages ORDER BY session_id').fetchall()
    if len(rows) > 4096: raise ValueError('ordinary voyage bound exceeded')
    sessions = []
    for session, encoded in rows:
        import uuid
        uuid.UUID(session)
        registration = json.loads(encoded)
        if registration.get('session_id') != session or registration.get('peer_uids') is not None:
            raise ValueError('legacy update requires ordinary registrations')
        directory = state/'sessions'/session
        stopped = json.loads(checked(directory/'stopped.json', 65536))
        guardian = json.loads(checked(directory/('guardian-'+registration['incarnation']+'.json'),65536))
        if any(record.get('session_id') != session or record.get('incarnation') != registration['incarnation'] or record.get('cleanup_observed') is not True for record in (stopped, guardian)):
            raise ValueError('ordinary voyage cleanup is not observed')
        journal = database(directory/'journal/journal.sqlite3')
        version = journal.execute('SELECT version FROM attachment_schema WHERE id=1').fetchone()[0]
        if journal.execute('SELECT count(*) FROM sessions WHERE id=?',(session,)).fetchone()[0] != 1:
            raise ValueError('legacy journal session identity missing')
        if not 2 <= version <= 12 or journal.execute('SELECT count(*) FROM runs WHERE active=1').fetchone()[0]:
            raise ValueError('legacy journal or active run prevents update')
        journal.close()
        sessions.append(session)
    return sessions

def tree_digest(root, state=False):
    sha, total, count = hashlib.sha256(), 0, 0
    if not root.exists(): raise ValueError('private source namespace missing')
    for path in sorted(root.rglob('*')):
        relative = path.relative_to(root).as_posix()
        info = path.lstat()
        if stat.S_ISDIR(info.st_mode):
            if info.st_uid != os.geteuid() or info.st_mode & 0o077: raise ValueError('private directory refused')
            continue
        if state and (relative in ('catalogue.sqlite3','catalogue.sqlite3-journal','process-http.json','update-quarantine.json') or (relative.startswith('sessions/') and len(relative.split('/'))==3 and relative.endswith('/registration.json'))):
            continue
        if stat.S_ISSOCK(info.st_mode): continue
        data = checked(path)
        total += len(data); count += 1
        if total > LIMIT or count > 16384: raise ValueError('private snapshot evidence bound exceeded')
        sha.update(relative.encode());sha.update(hashlib.sha256(data).digest())
    return sha.hexdigest()

def projection_matches(state, db):
    optional = {'executable', 'config_path', 'initialize', 'restart_from', 'peer_uids', 'name'}
    def normalized(record):
        return {key:value for key,value in record.items() if not (key in optional and value is None)}
    for session, encoded in db.execute('SELECT session_id,registration FROM voyages'):
        saved = json.loads(encoded)
        projected = json.loads(checked(state/'sessions'/session/'registration.json',65536))
        if normalized(saved) != normalized(projected):
            raise ValueError('ordinary registration projection changed')

def evidence(state, accounts, db):
    sessions=inventory(state,db)
    return dict(canonical_sha256=canonical(db), state_sha256=tree_digest(state,True),
                accounts_sha256=tree_digest(accounts), sessions=sessions, session_count=len(sessions))

def run(action,state,accounts,stage):
    state,accounts,stage=map(pathlib.Path,(state,accounts,stage))
    if any(not path.is_absolute() or path.resolve()!=path for path in (state,accounts,stage)):
        raise ValueError('noncanonical legacy namespace')
    db=database(state/'catalogue.sqlite3')
    version=db.execute('SELECT version FROM schema_version WHERE id=1').fetchone()[0]
    if version not in (1,2): raise ValueError('unsupported legacy catalogue source')
    current=evidence(state,accounts,db)
    projection_matches(state,db)
    if action=='inspect':
        if version != 1: raise ValueError('legacy preparation requires unchanged schema 1')
        return current
    proof_path=stage/'legacy-proof.json';backup=stage/'legacy-catalogue.sqlite3'
    if action=='snapshot':
        if version != 1: raise ValueError('legacy snapshot must precede migration')
        if backup.exists() or proof_path.exists(): raise ValueError('snapshot identity already exists')
        fd=os.open(backup,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600);os.close(fd)
        target=sqlite3.connect(backup);db.backup(target);target.close()
        fd=os.open(backup,os.O_RDONLY);os.fsync(fd);os.close(fd)
        current['backup_sha256']=digest_file(backup)
        fd=os.open(proof_path,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600)
        with os.fdopen(fd,'w') as stream:json.dump(current,stream,sort_keys=True);stream.flush();os.fsync(stream.fileno())
        fd=os.open(stage,os.O_RDONLY|os.O_DIRECTORY);os.fsync(fd);os.close(fd)
        return current
    supplied=sys.stdin.buffer.read(65537)
    if not supplied or len(supplied)>65536:raise ValueError('pinned legacy proof unavailable')
    expected=json.loads(supplied)
    saved=json.loads(checked(proof_path,512*1024))
    if not isinstance(expected,dict) or set(expected)!=(set(saved)-{'sessions'}):
        raise ValueError('pinned legacy proof shape changed')
    if any(saved[key]!=value for key,value in expected.items()):
        raise ValueError('staged proof does not match pinned approval')
    actual_backup=digest_file(backup)
    if actual_backup!=expected['backup_sha256']:raise ValueError('pinned legacy snapshot changed')
    if any(current[key]!=expected[key] for key in current if key!='sessions') or current['sessions']!=saved['sessions']:
        raise ValueError('post-snapshot state changed; legacy restore refused')
    current['backup_sha256']=actual_backup
    if action=='verify':return current
    if action!='restore':raise ValueError('unsupported legacy proof operation')
    db.close()
    # Caller holds supervisor/startup/execution locks and proved every writer stopped.
    # Restore the original SQLite snapshot, never mutate a schema-version row.
    destination=state/'catalogue.sqlite3';temporary=state/'catalogue.restore-exclusive'
    fd=os.open(temporary,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600)
    with os.fdopen(fd,'wb') as stream:stream.write(checked(backup,512*1024*1024));stream.flush();os.fsync(stream.fileno())
    os.replace(temporary,destination)
    sidecar=state/'catalogue.sqlite3-journal'
    if sidecar.exists():checked(sidecar,512*1024*1024);sidecar.unlink()
    fd=os.open(state,os.O_RDONLY|os.O_DIRECTORY);os.fsync(fd);os.close(fd)
    return current

if __name__=='__main__':
    try: print(json.dumps(run(*sys.argv[1:]),sort_keys=True))
    except Exception:
        print('Legacy snapshot or unchanged-state proof refused; no uncertain effects were replayed.',file=sys.stderr)
        raise SystemExit(1)
