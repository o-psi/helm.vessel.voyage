"""Private SOURCE contracts; no real manager, updater, native CT or signal."""
import hashlib
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import native_legacy_contexts as c


class Fixture:
    def __init__(self, root):
        self.HOME = Path(root)
        self.WORK = self.HOME/'q401';self.WORK.mkdir(mode=0o700)
        self.STATE = self.HOME/'state';self.STATE.mkdir(mode=0o700)
        self.UNIT = self.HOME/'unit.service';self.UNIT.write_bytes(b'[Service]\n');self.UNIT.chmod(0o600)
        self.NAME = 'voyage-vessel.service'
        self.environment = {'HOME':str(self.HOME),'XDG_RUNTIME_DIR':'/run/user/1000'}
        self.enabled = 'enabled';self.calls = [];self.evidence = {}
    @staticmethod
    def require(value, reason):
        if not value:raise RuntimeError(reason)
    def manager(self,*args):
        self.calls.append(args)
        if args == ('show-environment',):return '\n'.join(k+'='+v for k,v in self.environment.items())
        if args[0] == 'set-environment':
            key, value = args[1].split('=',1);self.environment[key] = value;return ''
        if args[0] == 'disable':self.enabled = 'disabled';return ''
        if args[0] == 'show':return self.enabled
        if args == ('daemon-reload',):return ''
        raise RuntimeError('unplanned manager effect')
    def write(self,name,value):
        if name in self.evidence:raise FileExistsError(name)
        self.evidence[name] = value
    def process_start(self,pid,*args,**kwargs):return 42
    @staticmethod
    def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
    ProcessNotLive = RuntimeError


class ContextContracts(unittest.TestCase):
    def fixture(self):
        temporary = tempfile.TemporaryDirectory();self.addCleanup(temporary.cleanup)
        return Fixture(temporary.name)

    def test_original_absent_and_explicit_namespace_values_remain_distinct(self):
        q = self.fixture()
        absent = c.manager_environment(q)
        self.assertNotIn('XDG_DATA_HOME',absent)
        q.environment['XDG_DATA_HOME'] = str(q.HOME/'.local/share')
        explicit = c.manager_environment(q)
        self.assertNotEqual(absent,explicit)

    def test_each_changed_manager_namespace_refuses_before_effect(self):
        for key in ('HOME','XDG_RUNTIME_DIR','XDG_DATA_HOME','XDG_CONFIG_HOME','XDG_STATE_HOME','XDG_CACHE_HOME'):
            with self.subTest(key=key):
                q = self.fixture();q.environment[key] = '/unreviewed'
                with self.assertRaises(RuntimeError):c.manager_environment(q)
                self.assertEqual(q.calls,[('show-environment',)])

    def test_namespace_change_records_true_absent_original_and_only_sets_owned_path(self):
        q = self.fixture();environment = c.manager_environment(q)
        result = c.change(q,'account-namespace',environment,b'',None,'enabled','context-account-namespace')
        self.assertFalse(result['original_present']);self.assertIsNone(result['original_value'])
        self.assertEqual(q.environment['XDG_DATA_HOME'],str(q.WORK/'independent-data'))
        self.assertFalse(any(call[0] in ('unset-environment','restart','start') for call in q.calls))

    def test_namespace_existing_destination_refuses_no_manager_mutation(self):
        q = self.fixture();(q.WORK/'independent-data').mkdir(mode=0o700)
        environment = c.manager_environment(q);q.calls.clear()
        with self.assertRaises(RuntimeError):c.change(q,'account-namespace',environment,b'',None,'enabled','x')
        self.assertEqual(q.calls,[])

    def test_persistent_enablement_change_is_real_and_never_reenabled(self):
        q = self.fixture()
        result = c.change(q,'enablement',{},b'',None,'enabled','x')
        self.assertEqual(result,{'original_unit_file_state':'enabled','changed_unit_file_state':'disabled'})
        self.assertEqual(q.calls,[('disable',q.NAME),('show',q.NAME,'--property=UnitFileState','--value')])
        q.calls.clear()
        for state in ('disabled','enabled-runtime','static','masked'):
            with self.subTest(state=state):
                with self.assertRaises(RuntimeError):c.change(q,'enablement',{},b'',None,state,'x')
                self.assertEqual(q.calls,[])

    def test_private_evidence_rejects_symlink_hardlink_public_mode_and_oversize(self):
        q = self.fixture();file = q.WORK/'private';file.write_bytes(b'owned');file.chmod(0o600)
        # Harness requires real mapped qualification UID. No fake foreign UID claim.
        with patch.object(c.os,'fstat',side_effect=lambda fd: self.meta(os.stat(file))):
            with patch.object(Path,'lstat',return_value=self.meta(os.stat(file))):
                self.assertEqual(c.private_bytes(q,file,5)[0],b'owned')
                with self.assertRaises(RuntimeError):c.private_bytes(q,file,4)
        link = q.WORK/'link';link.symlink_to(file)
        with self.assertRaises(OSError):c.private_bytes(q,link)
        for kind in ('public','hardlink'):
            with self.subTest(kind=kind):
                file.chmod(0o644 if kind == 'public' else 0o600)
                if kind == 'hardlink':os.link(file,q.WORK/'hard')
                with patch.object(c.os,'fstat',side_effect=lambda fd: self.meta(os.stat(file))):
                    with self.assertRaises(RuntimeError):c.private_bytes(q,file)

    @staticmethod
    def meta(value):
        # Only private fixture UID projection; all inode/mode/size/time fields are real.
        return SimpleNamespace(**{key:getattr(value,key) for key in (
            'st_dev','st_ino','st_mode','st_nlink','st_size','st_mtime_ns','st_ctime_ns')},st_uid=1000)

    def test_unit_comment_changes_same_owned_inode_and_saves_original_without_restore(self):
        q = self.fixture();original = q.UNIT.read_bytes();meta = self.meta(q.UNIT.stat())
        identity = (meta.st_dev,meta.st_ino,meta.st_uid,meta.st_mode,meta.st_nlink,meta.st_size,meta.st_mtime_ns,meta.st_ctime_ns)
        def read(_q,path,limit=1024*1024):
            m = self.meta(path.stat());return path.read_bytes(),(m.st_dev,m.st_ino,m.st_uid,m.st_mode,m.st_nlink,m.st_size,m.st_mtime_ns,m.st_ctime_ns)
        original_fstat = os.fstat
        with patch.object(c.os,'fstat',side_effect=lambda fd:self.meta(original_fstat(fd))), patch.object(c,'private_bytes',side_effect=read):
            result = c.change(q,'unit',{},original,identity,'enabled','context-unit')
        self.assertEqual((q.WORK/'context-unit-original.service').read_bytes(),original)
        self.assertEqual(q.UNIT.stat().st_ino,meta.st_ino)
        self.assertNotEqual(result['original_sha256'],result['changed_sha256'])
        self.assertEqual(q.calls,[('daemon-reload',)])

    def test_changed_unit_identity_refuses_before_write_or_reload(self):
        q = self.fixture();original = q.UNIT.read_bytes()
        with self.assertRaises(RuntimeError):c.change(q,'unit',{},original,('not-the-owned-inode',),'enabled','context-unit')
        self.assertEqual(q.UNIT.read_bytes(),original);self.assertEqual(q.calls,[])

    def test_stopped_requires_exact_start_and_kernel_stopped_state(self):
        q = self.fixture();owner = {'pid':123,'start_ticks':42}
        for state in ('T','t','S','R','Z'):
            raw = ('123 (fixture) '+state+' '+ ' '.join(['0']*18)+' 42').encode()
            with patch.object(c,'bounded',return_value=raw):self.assertEqual(c.stopped(q,owner),state in ('T','t'))
        with patch.object(c,'bounded',return_value=b'123 (fixture) T '+b'0 '*18+b'43'):
            with self.assertRaises(RuntimeError):c.stopped(q,owner)

    def test_admission_failure_retains_exclusive_result_without_any_signal(self):
        q = self.fixture();q.INSTALL = q.HOME/'install';(q.INSTALL/'updates').mkdir(parents=True)
        q.enabled = 'disabled';args = SimpleNamespace(context='enablement',installer_pid=None)
        with patch.object(c,'private_bytes',return_value=(b'unit',('pin',))),patch.object(c.signal,'pidfd_send_signal') as signal:
            with self.assertRaises(RuntimeError):c.run(q,args)
        signal.assert_not_called()
        self.assertEqual(q.evidence['context-enablement-result.json']['status'],'unqualified')
        self.assertFalse(q.evidence['context-enablement-result.json']['mutation_requested'])
        with self.assertRaises(FileExistsError):c.run(q,args)


    def test_snapshot_pins_exact_record_evidence_backup_and_quarantine(self):
        q = self.fixture();stage = q.HOME/'stage';stage.mkdir(mode=0o700)
        backup = b'private-original-db'
        evidence = {'backup_sha256':hashlib.sha256(backup).hexdigest(),'canonical_sha256':'original'}
        saved = {**evidence,'sessions':['fixture-session']}
        record = {'phase':'applying','operation_id':'op','current_release':'old','release_id':'new',
                  'legacy_accounts':str(q.HOME/'accounts'),'staging_root':str(stage),
                  'legacy_proof':{'state':str(q.STATE),'accounts':str(q.HOME/'accounts'),'stage':str(stage),'evidence':evidence}}
        quarantine = {'schema_version':1,'operation_id':'op','previous_release':'old','candidate_release':'new'}
        q.installed_bin = lambda:q.HOME/'old'/'bin'
        q.sqlite_observe = lambda *args:[(1,)]
        def data(_q,path,limit=0):
            value = backup if path.name == 'legacy-catalogue.sqlite3' else json.dumps(saved if path.name == 'legacy-proof.json' else quarantine).encode()
            return value,('private-inode',path.name)
        with patch.object(c,'private_bytes',side_effect=data):
            self.assertEqual(c.snapshot_pin(q,record)['backup']['sha256'],evidence['backup_sha256'])
            quarantine['candidate_release'] = 'another'
            with self.assertRaises(RuntimeError):c.snapshot_pin(q,record)
            quarantine['candidate_release'] = 'new';saved['canonical_sha256'] = 'changed'
            with self.assertRaises(RuntimeError):c.snapshot_pin(q,record)
            saved['canonical_sha256'] = 'original';record['phase'] = 'committing'
            with self.assertRaises(RuntimeError):c.snapshot_pin(q,record)
            record['phase'] = 'applying';q.sqlite_observe = lambda *args:[(2,)]
            with self.assertRaises(RuntimeError):c.snapshot_pin(q,record)
        self.assertEqual(q.calls,[])

    def test_every_owner_thread_must_have_no_helper_or_zombie_child(self):
        q = self.fixture();owner = {'pid':321}
        with patch.object(Path,'iterdir',return_value=iter([Path('/proc/321/task/321'),Path('/proc/321/task/322')])):
            with patch.object(c,'bounded',side_effect=[b'',b'591']):
                with self.assertRaises(RuntimeError):c.no_children(q,owner)
        with patch.object(Path,'iterdir',return_value=iter([Path('/proc/321/task/321')])):
            with patch.object(c,'bounded',return_value=b''):c.no_children(q,owner)

    def test_updater_namespace_requires_exact_account_and_no_duplicate_keys(self):
        q = self.fixture();owner = {'pid':321}
        valid = ('HOME='+str(q.HOME)+'\0XDG_RUNTIME_DIR=/run/user/1000\0').encode()
        with patch.object(c,'bounded',return_value=valid):self.assertEqual(c.owner_namespace(q,owner)['HOME'],str(q.HOME))
        for raw in (valid+b'HOME=/another\0',valid+b'XDG_DATA_HOME=/another\0',b'HOME=/another\0'):
            with patch.object(c,'bounded',return_value=raw):
                with self.assertRaises(RuntimeError):c.owner_namespace(q,owner)

    def test_continuation_requires_live_state_and_post_state_exact_identity(self):
        q = self.fixture();owner = {'pid':321,'start_ticks':42};environment = {'HOME':str(q.HOME)}
        for state in ('T','t','Z','X','x','unknown'):
            with self.subTest(state=state),patch.object(c,'owner_state',return_value=state),patch.object(c,'exact_again') as exact:
                self.assertFalse(c.continued_now(q,owner,Path('/fixture'),[],environment));exact.assert_not_called()
        for state in ('R','S','D','I'):
            with self.subTest(state=state),patch.object(c,'owner_state',return_value=state),patch.object(c,'exact_again',return_value=True),patch.object(c,'owner_namespace',return_value=environment):
                self.assertTrue(c.continued_now(q,owner,Path('/fixture'),[],environment))
        with patch.object(c,'owner_state',return_value='R'),patch.object(c,'exact_again',return_value=False):
            with self.assertRaises(RuntimeError):c.continued_now(q,owner,Path('/fixture'),[],environment)
        with patch.object(c,'owner_state',return_value='R'),patch.object(c,'exact_again',return_value=True),patch.object(c,'owner_namespace',return_value={'HOME':'/changed'}):
            with self.assertRaises(RuntimeError):c.continued_now(q,owner,Path('/fixture'),[],environment)

    def orchestration(self, conflict=False):
        q = self.fixture();q.INSTALL = q.HOME/'install';updates = q.INSTALL/'updates';updates.mkdir(parents=True)
        release = q.WORK/'candidate'/'release';(release/'bin').mkdir(parents=True)
        image = release/'bin'/'voyage-installer';image.write_bytes(b'qualified-fixture-image')
        (release/'release.json').write_text(json.dumps({'binaries':{'voyage-installer':{'sha256':q.digest(image)}}}))
        (q.WORK/'candidate-bin.json').write_text(json.dumps(str(image)))
        record = {'operation_id':'fixture-operation','created_at':int(c.time.time()),'legacy_proof':{'fixture':True}}
        original_write = q.write
        def write(name,value):
            original_write(name,value)
            if name.endswith('-attempt.json'):(updates/'fresh.json').write_text(json.dumps(record))
        q.write = write
        owner = {'descriptor':91,'pid':321,'start_ticks':42,'uid':1000,'executable_sha256':q.digest(image)}
        owner_args = [os.fsencode(image),b'--bin-dir',os.fsencode(image.parent),b'upgrade',b'--no-start']
        args = SimpleNamespace(context='enablement',installer_pid=321)
        from contextlib import ExitStack
        with ExitStack() as stack:
            stack.enter_context(patch.object(c,'private_bytes',return_value=(b'unit',('unit-pin',))))
            stack.enter_context(patch.object(c,'record_pin',return_value={'operation_id':'fixture-operation','created_at':record['created_at'],'supervisor_activation':{'enabled':True,'unit_file_state':'enabled','definition':'unit'}}))
            stack.enter_context(patch.object(c,'stable_record',return_value=record))
            stack.enter_context(patch.object(c,'argv',return_value=owner_args))
            stack.enter_context(patch.object(c,'process_witness',return_value=('verified',owner)))
            stack.enter_context(patch.object(c,'exact_again',return_value=True))
            stack.enter_context(patch.object(c,'no_children'))
            stack.enter_context(patch.object(c,'owner_namespace',return_value={'HOME':str(q.HOME)}))
            stack.enter_context(patch.object(c,'snapshot_pin',side_effect=[{'pin':1},{'pin':2 if conflict else 1}]))
            stack.enter_context(patch.object(c,'stopped',side_effect=[True,True] if conflict else [True,True,True]))
            stack.enter_context(patch.object(c,'continued_now',return_value=True))
            close = stack.enter_context(patch.object(c.os,'close'))
            signals = stack.enter_context(patch.object(c.signal,'pidfd_send_signal'))
            mutation = stack.enter_context(patch.object(c,'change',return_value={'observed':'disabled'}))
            if conflict:
                with self.assertRaises(RuntimeError):c.run(q,args)
            else:c.run(q,args)
            self.assertEqual(signals.call_args_list,[unittest.mock.call(91,c.signal.SIGSTOP),unittest.mock.call(91,c.signal.SIGCONT)])
            self.assertEqual(mutation.call_count,0 if conflict else 1)
            close.assert_called_once_with(91)
        return q.evidence['context-enablement-result.json']

    def test_one_retained_pause_one_context_change_and_one_positive_continue(self):
        result = self.orchestration()
        self.assertEqual(result['status'],'context_changed_and_owner_continued')
        self.assertTrue(result['same_owner_stopped_observed']);self.assertTrue(result['same_owner_continued_observed'])
        self.assertFalse(result['rollback_or_cleanup_acceptance']);self.assertFalse(result['implicit_restoration'])

    def test_snapshot_conflict_after_pause_resumes_same_owner_without_mutation(self):
        result = self.orchestration(conflict=True)
        self.assertEqual(result['status'],'unqualified');self.assertFalse(result['mutation_requested'])
        self.assertTrue(result['same_owner_continued_observed']);self.assertFalse(result['mutation_observed'])


if __name__ == '__main__':unittest.main()
