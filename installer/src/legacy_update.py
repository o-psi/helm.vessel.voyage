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

def leased(path):
    import fcntl
    info=path.stat()
    descriptors=os.environ.get('LEGACY_UPDATE_LOCK_FDS','').split(',')
    if len(descriptors)>16385:raise ValueError('helper lease bound exceeded')
    for raw in descriptors:
        if not raw:continue
        descriptor=int(raw); held=os.fstat(descriptor)
        if (info.st_dev,info.st_ino)==(held.st_dev,held.st_ino):
            fcntl.flock(descriptor,fcntl.LOCK_EX|fcntl.LOCK_NB)
            return True
    return False

def guardian_idle(directory):
    import fcntl
    path=directory/'guardian.lock'
    if path.exists() and leased(path):return
    fd=os.open(path,os.O_CREAT|os.O_RDWR|os.O_NOFOLLOW|os.O_CLOEXEC,0o600)
    try:fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
    finally:os.close(fd)

def inventory(state, db, journal_ceiling=12):
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
        guardian_idle(directory)
        if set(guardian)!={'session_id','incarnation','boot_id','cleanup_observed'}:
            raise ValueError('strict guardian evidence unavailable')
        uuid.UUID(guardian['boot_id'])
        current_boot=pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip()
        guardian_clean=guardian['cleanup_observed'] is True or guardian['boot_id']!=current_boot
        if stopped.get('session_id')!=session or stopped.get('incarnation')!=registration['incarnation'] or stopped.get('cleanup_observed') is not True or not guardian_clean:
            raise ValueError('ordinary voyage cleanup is not observed')
        if any(record.get('session_id') != session or record.get('incarnation') != registration['incarnation'] for record in (stopped, guardian)):
            raise ValueError('ordinary voyage cleanup is not observed')
        journal = database(directory/'journal/journal.sqlite3')
        version = journal.execute('SELECT version FROM attachment_schema WHERE id=1').fetchone()[0]
        owned=journal.execute('SELECT count(*) FROM sessions WHERE id=?',(session,)).fetchone()[0]
        if owned!=1:
            empty=journal.execute('SELECT count(*) FROM sessions').fetchone()[0]==0 and all(journal.execute('SELECT count(*) FROM '+table).fetchone()[0]==0 for table in ('runs','commands','events'))
            if not empty or stopped.get('startup_failed') is not True:
                raise ValueError('legacy journal session identity missing')
        if not 2 <= version <= journal_ceiling or journal.execute('SELECT count(*) FROM runs WHERE active=1').fetchone()[0]:
            raise ValueError('legacy journal or active run prevents update')
        journal.close()
        sessions.append(session)
    return sessions

# Exact schema-1 initialization from Vessel notifications/store.rs::Store::open.
# Comparing the complete engine schema and column metadata rejects extra objects,
# triggers, virtual tables and silently changed definitions before reading rows.
NOTIFICATION_SCHEMA = """
CREATE TABLE clock (id INTEGER PRIMARY KEY CHECK(id=1), now INTEGER NOT NULL);
INSERT INTO clock VALUES(1,0);
CREATE TABLE destinations (
 id TEXT PRIMARY KEY, grant_id TEXT NOT NULL, source_id TEXT NOT NULL,
 payload TEXT NOT NULL CHECK(length(payload)<=1024), expires INTEGER NOT NULL,
 revoked INTEGER, accepted INTEGER, cursor INTEGER NOT NULL DEFAULT 0, producer_error TEXT);
CREATE TABLE commands (
 id TEXT PRIMARY KEY, destination_id TEXT NOT NULL REFERENCES destinations(id),
 operation INTEGER NOT NULL CHECK(operation BETWEEN 0 AND 3), argument TEXT);
CREATE UNIQUE INDEX lifecycle_commands ON commands(destination_id,operation) WHERE operation<3;
CREATE TABLE events (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 destination_id TEXT NOT NULL REFERENCES destinations(id), event_id TEXT NOT NULL,
 source_event_id TEXT NOT NULL, fingerprint BLOB NOT NULL CHECK(length(fingerprint)=32),
 payload TEXT CHECK(length(payload)<=1024), expires INTEGER NOT NULL,
 state INTEGER NOT NULL DEFAULT 0 CHECK(state BETWEEN 0 AND 2),
 UNIQUE(destination_id,event_id), UNIQUE(destination_id,source_event_id));
CREATE INDEX inbox ON events(destination_id,sequence);
PRAGMA user_version=1;
"""
NOTIFICATION_FILES = ('notifications/notifications.sqlite3',
                      'notifications/notifications.sqlite3-journal')

def notification_schema(db):
    return [(kind,name,table,' '.join(sql.split()) if sql is not None else None)
            for kind,name,table,sql in db.execute(
                'SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name')]

def cold_notification_journal(path):
    data=checked(path,1024*1024)
    if data and (len(data)<28 or data[:28]!=bytes(28)):
        raise ValueError('notification journal is hot or unsupported')
    return data

def notification_projection(state):
    directory=state/'notifications'
    info=directory.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid!=os.geteuid() or info.st_mode&0o077:
        raise ValueError('private notification directory unavailable')
    path=directory/'notifications.sqlite3'
    checked(path,64*1024*1024)
    cold_notification_journal(directory/'notifications.sqlite3-journal')
    identity=path.lstat()
    db=database(path)
    try:
        if db.execute('PRAGMA user_version').fetchone()!=(1,):
            raise ValueError('unsupported notification review schema')
        expected=sqlite3.connect(':memory:')
        try:
            expected.executescript(NOTIFICATION_SCHEMA)
            metadata={name:db.execute('PRAGMA '+name).fetchone()[0] for name in
                      ('user_version','application_id','encoding','page_size','auto_vacuum','schema_version')}
            expected_metadata={name:expected.execute('PRAGMA '+name).fetchone()[0] for name in metadata}
            if metadata!=expected_metadata or db.execute('PRAGMA page_count').fetchone()[0]>16384:
                raise ValueError('notification database metadata unsupported')
            schema=notification_schema(db)
            if schema!=notification_schema(expected):
                raise ValueError('notification schema objects changed')
            sha=hashlib.sha256(json.dumps([metadata,schema],sort_keys=True,separators=(',',':')).encode())
            total=count=0
            for table in ('clock','commands','destinations','events','sqlite_sequence'):
                columns=db.execute('PRAGMA table_xinfo('+table+')').fetchall()
                if columns!=expected.execute('PRAGMA table_xinfo('+table+')').fetchall():
                    raise ValueError('notification columns changed')
                sha.update(json.dumps([table,columns],separators=(',',':')).encode())
                for row in db.execute('SELECT rowid,* FROM '+table+' ORDER BY rowid'):
                    for column,value in zip(columns,row[1:]):
                        _,name,kind,nonnull,_,primary,_=column
                        if table=='sqlite_sequence':kind={'name':'TEXT','seq':'INTEGER'}[name]
                        if value is None:
                            if nonnull or primary:raise ValueError('unsupported null notification field')
                        elif (kind=='INTEGER' and not isinstance(value,int) or
                              kind=='TEXT' and not isinstance(value,str) or
                              kind=='BLOB' and not isinstance(value,bytes)):
                            raise ValueError('unsupported typed notification field')
                    typed=[]
                    for value in row:
                        if value is None:typed.append(['null',None])
                        elif isinstance(value,int):typed.append(['integer',value])
                        elif isinstance(value,float):typed.append(['real',value.hex()])
                        elif isinstance(value,str):typed.append(['text',value])
                        elif isinstance(value,bytes):typed.append(['blob',value.hex()])
                        else:raise ValueError('unsupported notification value')
                    encoded=json.dumps(typed,ensure_ascii=False,separators=(',',':')).encode()
                    total+=len(encoded);count+=1
                    if total>LIMIT or count>65536:raise ValueError('notification review bound exceeded')
                    sha.update(encoded)
            clock=db.execute('SELECT id,now FROM clock').fetchall()
            if len(clock)!=1 or clock[0][0]!=1 or not isinstance(clock[0][1],int) or clock[0][1]<0:
                raise ValueError('notification clock record unsupported')
            cold_notification_journal(directory/'notifications.sqlite3-journal')
            current=path.lstat()
            if (current.st_dev,current.st_ino)!=(identity.st_dev,identity.st_ino):
                raise ValueError('notification database namespace changed')
            return db,sha.hexdigest(),(identity.st_dev,identity.st_ino)
        finally:expected.close()
    except BaseException:
        db.close()
        raise

def notification_review_tree(state):
    db,projection,identity=notification_projection(state)
    try:
        result=tree_digest(state,True,projection)
        cold_notification_journal(state/NOTIFICATION_FILES[1])
        current=(state/NOTIFICATION_FILES[0]).lstat()
        if (current.st_dev,current.st_ino)!=identity:raise ValueError('notification database namespace changed')
        return result
    finally:db.close()

def tree_digest(root, state=False, notification_projection=None):
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
        sha.update(relative.encode())
        if notification_projection is not None and relative in NOTIFICATION_FILES:
            # Every file still contributes name, existence, privacy and size bounds.
            # Only the cold pair's physical layout is normalized for owner review.
            normalized = ('notification-schema1:'+notification_projection if relative==NOTIFICATION_FILES[0]
                          else 'notification-cold-persist-journal')
            sha.update(hashlib.sha256(normalized.encode()).digest())
        else:sha.update(hashlib.sha256(data).digest())
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

def evidence(state, accounts, db, journal_ceiling=12):
    sessions=inventory(state,db,journal_ceiling)
    return dict(canonical_sha256=canonical(db), state_sha256=tree_digest(state,True),
                accounts_sha256=tree_digest(accounts), sessions=sessions, session_count=len(sessions))

def run(action,state,accounts,stage):
    state,accounts,stage=map(pathlib.Path,(state,accounts,stage))
    if any(not path.is_absolute() or path.resolve()!=path for path in (state,accounts,stage)):
        raise ValueError('noncanonical legacy namespace')
    db=database(state/'catalogue.sqlite3')
    version=db.execute('SELECT version FROM schema_version WHERE id=1').fetchone()[0]
    if version not in (1,2): raise ValueError('unsupported legacy catalogue source')
    forward=action.startswith('forward-')
    if forward and version!=2:raise ValueError('forward recovery requires existing schema 2')
    current=evidence(state,accounts,db,20 if forward else 12)
    if forward:
        current['recovery_mode']='forward-existing-schema2'
        current['review_state_sha256']=notification_review_tree(state)
    projection_matches(state,db)
    if action in ('inspect','forward-inspect'):
        if not forward and version != 1: raise ValueError('legacy preparation requires unchanged schema 1')
        return current
    proof_path=stage/'legacy-proof.json';backup=stage/'legacy-catalogue.sqlite3'
    if action in ('snapshot','forward-snapshot'):
        if not forward and version != 1: raise ValueError('legacy snapshot must precede migration')
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
    if not forward and 'recovery_mode' in expected:raise ValueError('forward evidence cannot authorize legacy restore')
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
    if action=='verify-restored':
        if version!=1:raise ValueError('previous snapshot schema not restored')
        restored=json.loads(checked(stage/'legacy-restored.json',65536))
        if restored!=expected:raise ValueError('previous snapshot restoration marker not pinned')
        return current
    if action in ('verify','forward-verify'):return current
    if action!='restore':raise ValueError('unsupported legacy proof operation')
    if not leased(state/'supervisor.lock') or any(not leased(state/'sessions'/session/'startup.lock') or not leased(state/'sessions'/session/'guardian.lock') or not leased(state/'sessions'/session/'journal'/(session+'.execution.lock')) for session in current['sessions']):
        raise ValueError('helper-held restoration ownership unavailable')
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
    marker=stage/'legacy-restored.json'
    descriptor=os.open(marker,os.O_CREAT|os.O_EXCL|os.O_WRONLY|os.O_NOFOLLOW,0o600)
    with os.fdopen(descriptor,'w') as stream:json.dump(expected,stream,sort_keys=True);stream.flush();os.fsync(stream.fileno())
    fd=os.open(stage,os.O_RDONLY|os.O_DIRECTORY);os.fsync(fd);os.close(fd)
    return current

if __name__=='__main__':
    try: print(json.dumps(run(*sys.argv[1:]),sort_keys=True))
    except Exception:
        print('Legacy snapshot or unchanged-state proof refused; no uncertain effects were replayed.',file=sys.stderr)
        raise SystemExit(1)
