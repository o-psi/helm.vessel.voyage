"""Prepared private selection/principal contracts, no Native or TLS effects."""
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import unittest
import uuid
import sys
import time
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parent))
import host_browser_production as launcher
import host_browser_production_probe as probe


class NativeLocalContracts(unittest.TestCase):
    def fixture(self):
        temporary=tempfile.TemporaryDirectory();self.addCleanup(temporary.cleanup)
        path=Path(temporary.name);path.chmod(0o700)
        return path

    def test_local_uses_existing_metadata_and_normal_directory_without_parsing_or_copying_token(self):
        directory=self.fixture();discovery=directory/'process-http.json'
        discovery.write_text('private fixture, deliberately not a credential JSON');discovery.chmod(0o600)
        config={'native_route':{'mode':'local','directory':str(directory),'expected_vessel_id':str(uuid.uuid4())}}
        route,args,pin=launcher.native_selection(config)
        self.assertEqual(args,['--no-start','--directory',str(directory)])
        self.assertEqual(route,config['native_route']);self.assertEqual(pin[:2],(discovery.stat().st_dev,discovery.stat().st_ino))
        self.assertEqual(discovery.read_text(),'private fixture, deliberately not a credential JSON')

    def test_public_existing_selection_is_preserved_and_ambiguous_local_config_refuses(self):
        directory=self.fixture();access=directory/'access.json';access.write_text('retained');access.chmod(0o600)
        route,args,pin=launcher.native_selection({'access_file':str(access)})
        self.assertEqual(route,{'mode':'public'});self.assertEqual(args,['--no-start','--access-file',str(access)]);self.assertIsNone(pin)
        with self.assertRaises(AssertionError):launcher.native_selection({'native_route':{'mode':'local','directory':str(directory),'expected_vessel_id':str(uuid.uuid4())},'access_file':str(access)})
        with self.assertRaises(AssertionError):launcher.native_selection({'native_route':{'mode':'replacement'}})

    def test_local_discovery_symlink_public_file_and_nil_vessel_refuse(self):
        directory=self.fixture();file=directory/'original';file.write_text('retained');file.chmod(0o600)
        discovery=directory/'process-http.json';discovery.symlink_to(file)
        config={'native_route':{'mode':'local','directory':str(directory),'expected_vessel_id':str(uuid.uuid4())}}
        with self.assertRaises(OSError):launcher.native_selection(config)
        discovery.unlink();discovery.write_text('fixture');discovery.chmod(0o644)
        with self.assertRaises(AssertionError):launcher.native_selection(config)
        discovery.chmod(0o600);config['native_route']['expected_vessel_id']=str(uuid.UUID(int=0))
        with self.assertRaises(AssertionError):launcher.native_selection(config)

    def authority(self):
        directory=self.fixture();sessions=[]
        for label in ('a','b'):
            sid=str(uuid.uuid4());root=directory/'sessions'/sid/'journal/host-browser';root.mkdir(parents=True,mode=0o700)
            for parent in (root.parent,root.parent.parent):parent.chmod(0o700)
            identity=root.parent.parent/'identity';identity.mkdir(mode=0o700)
            actor={'version':1,'actor':{'installation_id':str(uuid.uuid4()),'principal_id':str(uuid.uuid4())}}
            file=identity/'actor.json';file.write_text(json.dumps(actor));file.chmod(0o600)
            receipt=str(uuid.uuid4());socket=str(uuid.uuid4())
            claim={'action':'attach','command_id':receipt,'binding':{key:str(uuid.uuid4()) for key in ('incarnation','browser_id','attachment_id','tab_id')}}
            claim['binding'].update(document_epoch=1,viewport_epoch=1,controller_epoch=1,capture_epoch=1)
            digest=__import__('hashlib').sha256(json.dumps({'operation':claim,'socket':socket,'principal':actor['actor']['principal_id']},sort_keys=True,separators=(',',':')).encode()).hexdigest()
            db=sqlite3.connect(root/'receipts.sqlite3')
            db.execute('CREATE TABLE receipts(id TEXT PRIMARY KEY,principal TEXT,digest TEXT,state TEXT)')
            db.execute('INSERT INTO receipts VALUES(?,?,?,?)',(receipt,actor['actor']['principal_id'],digest,'completed'));db.commit();db.close();(root/'receipts.sqlite3').chmod(0o600)
            sessions.append({'label':label,'session_id':sid,'socket_id':socket,'root':str(root),'claims':[claim]})
        vessel=str(uuid.uuid4());public_dir=directory/'identity';public_dir.mkdir(mode=0o700)
        public=public_dir/'public.json';public.write_text(json.dumps({'vessel_id':vessel,'public_key':__import__('base64').b64encode(b'p'*32).decode()}));public.chmod(0o600)
        return {'schema':1,'directory':str(directory),'vessel_id':vessel,'sessions':sessions}

    def test_actual_private_sqlite_completed_receipts_match_existing_runtime_actor(self):
        request=self.authority();before=[Path(item['root']).joinpath('receipts.sqlite3').read_bytes() for item in request['sessions']]
        result=probe.local_authority(request)
        self.assertEqual(result['mode'],'local');self.assertTrue(result['no_effects']);self.assertEqual(len(result['identities']),2)
        self.assertEqual([item['completed_native_receipt_ids'] for item in result['identities']],[[claim['command_id'] for claim in item['claims']] for item in request['sessions']])
        self.assertEqual(before,[Path(item['root']).joinpath('receipts.sqlite3').read_bytes() for item in request['sessions']])

    def test_wrong_unknown_receipt_principal_state_root_or_socket_never_supplies_attribution(self):
        for fault in ('principal','state','missing','root','socket'):
            request=self.authority();item=request['sessions'][0]
            if fault in ('principal','state'):
                db=sqlite3.connect(Path(item['root'])/'receipts.sqlite3')
                if fault=='principal':db.execute('UPDATE receipts SET principal=?',(str(uuid.uuid4()),))
                else:db.execute("UPDATE receipts SET state='unknown'")
                db.commit();db.close()
            elif fault=='missing':item['claims'][0]['command_id']=str(uuid.uuid4())
            elif fault=='root':item['root']=request['sessions'][1]['root']
            else:item['socket_id']=str(uuid.UUID(int=0))
            with self.subTest(fault=fault),self.assertRaises(AssertionError):probe.local_authority(request)

    def test_changed_valid_socket_or_binding_fails_exact_recorded_operation_digest(self):
        for field in ('socket','binding'):
            request=self.authority();item=request['sessions'][0]
            if field=='socket':item['socket_id']=str(uuid.uuid4())
            else:item['claims'][0]['binding']['controller_epoch']+=1
            with self.subTest(field=field),self.assertRaises(AssertionError):probe.local_authority(request)

    def test_actor_proof_independently_refuses_wrong_physical_public_vessel_identity(self):
        request=self.authority();request['vessel_id']=str(uuid.uuid4())
        with self.assertRaises(AssertionError):probe.local_authority(request)


class NativeBootstrapContracts(unittest.TestCase):
    """Callbacks only: authentication/selection order and refusal before real F6."""
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.root.chmod(0o700)
        self.item={'id':str(uuid.uuid4()),'label':'a','title':'qualification-333-a-title-clipped-in-sidebar','workspace':'/fixture-a'}
        self.vessel=str(uuid.uuid4());self.socket=str(uuid.uuid4())
        self.config={'native_route':{'mode':'local','directory':'/private/vessel','expected_vessel_id':self.vessel},
                     'native_helm_program':{'path':'/qualified/helm','sha256':'a'*64}}
        self.metrics=self.root/'meter.json';self.capture=self.root/'capture'
        self.value={'status':'observed','pid':901,'start_ticks':500,'label':'native-a','routes':1,'active':1,
            'connections':1,'attempts':1,'disconnected':0,'handshake_failures':0,'transport_failures':0,
            'socket_id':self.socket,'vessel_id':self.vessel,'transport':'local_ws','authority':'existing local account owner'}
        self.write_meter()
        self.actions=[];self.phase='initial';self.draft='';self.wrong=None;self.swallow=False;self.fresh=True
        self.owner={'session_id':self.item['id'],'incarnation':str(uuid.uuid4()),'revision':7}
        self.ident=(500,'/qualified/helm')
        self.patch=patch.object(launcher,'process_identity',side_effect=lambda _:self.ident);self.patch.start();self.addCleanup(self.patch.stop)
        class Process:
            def poll(self):return None
        self.client={'pid':901,'process':Process()}
    def write_meter(self):
        self.value['captured_at_ms']=int(time.time()*1000)
        self.metrics.write_text(json.dumps(self.value));self.metrics.chmod(0o600)
    def screen(self,_):
        if self.phase=='settings':
            session=str(uuid.uuid4()) if self.wrong=='session' else self.item['id']
            route=str(uuid.uuid4()) if self.wrong=='route' else launcher.native_route_id(self.config)
            return ('Effective settings · read-only observations\nVoyage '+session+' · Vessel connection '+route+
                    '\nSource: last authenticated executing-host snapshot (not a new fetch).\nRevision 7 · access: approval')
        if self.phase=='browser':return 'Host browser\nVoyage: '+self.item['id']
        # No phase presents the complete fixture title. Initial selection is an
        # unrelated voyage; the normal /use still must precede settings and F6.
        return 'Other voyage (fixture title clipped)\nAsk anything, or describe what you want to do...\nEnter Submit new turn'
    def paste(self,_,value):
        self.assertTrue((self.root/'native-a-bootstrap-intent.json').exists())
        self.actions.append(('paste',value));self.draft=value
    def send(self,_,value):
        self.actions.append(('send',value))
        if value=='\r':
            if self.draft=='/use '+self.item['id']:self.phase='selected'
            elif self.draft=='/settings':self.phase='settings'
            self.draft=''
        if value=='\x1b' and not self.swallow:self.phase='selected'
        if value=='\x1b[17~':
            self.assertTrue((self.root/'native-a-bootstrap-selected.json').exists())
            if self.fresh:self.capture.write_text('/private/open.html');self.capture.chmod(0o600)
            self.phase='browser'
    def wait(self,predicate,seconds):
        self.assertLessEqual(seconds,40)
        value=predicate();self.assertTrue(value,'bounded callback observation unavailable');return value
    def run_bootstrap(self):
        return launcher.bootstrap_native(self.client,self.item,self.config,self.metrics,self.capture,self.root,
            lambda:dict(self.owner),lambda:None,self.screen,self.paste,self.send,self.wait)
    def result(self):return json.loads((self.root/'native-a-bootstrap-result.json').read_text())
    def test_clipped_unselected_title_selects_exact_sid_and_readonly_route_revision_before_f6(self):
        value=self.run_bootstrap()
        self.assertEqual(value,{'pid':901,'start_ticks':500,'label':'native-a','descendants':True})
        self.assertEqual(self.actions,[('paste','/use '+self.item['id']),('send','\r'),('paste','/settings'),('send','\r'),('send','\x1b'),('send','\x1b[17~')])
        self.assertEqual(self.result()['state'],'observed');self.assertEqual(self.result()['selected_owner'],self.owner)
    def test_wrong_actual_selected_session_prevents_dismissal_and_f6(self):
        self.wrong='session'
        with self.assertRaises(AssertionError):self.run_bootstrap()
        self.assertEqual(self.result()['state'],'unknown');self.assertFalse(self.result()['f6_dispatched'])
        self.assertNotIn(('send','\x1b[17~'),self.actions)
    def test_wrong_actual_route_refuses_before_f6(self):
        self.wrong='route'
        with self.assertRaises(AssertionError):self.run_bootstrap()
        self.assertFalse(self.result()['f6_dispatched'])
    def test_changed_canonical_revision_refuses_before_f6(self):
        self.owner['revision']=8
        with self.assertRaises(AssertionError):self.run_bootstrap()
        self.assertFalse(self.result()['f6_dispatched'])
    def test_changed_native_socket_refuses_before_next_key(self):
        original=self.send
        def changed(client,value):
            original(client,value)
            if value=='\r':self.value['socket_id']=str(uuid.uuid4());self.write_meter()
        self.send=changed
        with self.assertRaises(AssertionError):self.run_bootstrap()
        self.assertEqual(self.actions,[('paste','/use '+self.item['id']),('send','\r')])
        self.assertFalse(self.result()['f6_dispatched'])
    def test_changed_native_process_start_refuses_before_next_key(self):
        original=self.send
        def changed(client,value):
            original(client,value)
            if value=='\r':self.ident=(501,'/qualified/helm')
        self.send=changed
        with self.assertRaises(AssertionError):self.run_bootstrap()
        self.assertEqual(len(self.actions),2);self.assertFalse(self.result()['f6_dispatched'])
    def test_settings_overlay_not_dismissed_prevents_f6(self):
        self.swallow=True
        with self.assertRaises(AssertionError):self.run_bootstrap()
        self.assertFalse(self.result()['f6_dispatched']);self.assertNotIn(('send','\x1b[17~'),self.actions)
    def test_missing_one_use_launcher_after_single_f6_retains_unknown(self):
        self.fresh=False
        with self.assertRaises(AssertionError):self.run_bootstrap()
        self.assertTrue(self.result()['f6_dispatched']);self.assertEqual(self.actions.count(('send','\x1b[17~')),1)
        self.assertEqual(self.result()['state'],'unknown')
    def test_changed_meter_inode_refuses_even_when_scalar_identity_is_copied(self):
        original=self.send
        def changed(client,value):
            original(client,value)
            if value=='\r':
                replacement=self.root/'replacement';replacement.write_text(json.dumps(self.value));replacement.chmod(0o600);replacement.replace(self.metrics)
        self.send=changed
        with self.assertRaises(AssertionError):self.run_bootstrap()
        self.assertEqual(len(self.actions),2);self.assertFalse(self.result()['f6_dispatched'])
    def test_fixture_observation_rejects_duplicate_or_changed_owner_metadata(self):
        process={'session_id':self.item['id'],'incarnation':self.owner['incarnation'],'name':self.item['title'],'workspace':self.item['workspace']}
        snapshot={'session_id':self.item['id'],'name':self.item['title'],'revision':7,'messages':[],'run':None,'pending_cleanup_run':None}
        self.assertEqual(launcher.fixture_observation([process],snapshot,self.item),self.owner)
        for catalogue,snap in [([process,process],snapshot),([{**process,'workspace':'/different'}],snapshot),
                              ([{**process,'incarnation':str(uuid.UUID(int=0))}],snapshot),([process],{**snapshot,'run':{'state':'running'}})]:
            with self.assertRaises(AssertionError):launcher.fixture_observation(catalogue,snap,self.item)

    def settings_terminal(self, session=None, route=None, stale=False):
        # render.rs::draw calls draw_discovery_help with the full 120x36 area.
        # That renderer insets x2/y1, wraps at116 display cells and adds ONLY a
        # bottom border. It does not use the boxed right-panel renderer.
        body=['Effective settings · read-only observations','',
              'Voyage '+(session or self.item['id'])+' · Vessel connection '+(route or launcher.native_route_id(self.config)),
              'STALE: executing Vessel disconnected; these are last observed values.' if stale else
              'Source: last authenticated executing-host snapshot (not a new fetch).',
              'Revision 7 · access: approval']
        rows=[' '*120 for _ in range(36)]
        for row,line in enumerate(body,1):
            self.assertLessEqual(len(line),116)
            rows[row]='  '+line.ljust(116)+'  '
        rows[34]='  '+('─'*116)+'  '
        raw=bytearray(b'\x1b[2J')
        for row,line in enumerate(rows,1):raw.extend(('\x1b['+str(row)+';1H'+line).encode())
        return launcher.screen({'rows':36,'columns':120,'output':raw})

    def test_actual_full_area_settings_geometry_and_terminal_cells_preserve_exact_identity(self):
        frame=self.settings_terminal()
        self.assertEqual(len(frame.splitlines()),36)
        self.assertTrue(all(len(line)==120 for line in frame.splitlines()))
        self.assertEqual(launcher.settings_selection(frame,self.item['id'],launcher.native_route_id(self.config)),7)
        # The 100-character identity line fits the actual116-cell body. Border
        # glyphs are retained by screen(), but occur on the bottom line only.
        self.assertEqual(len(frame.splitlines()[3].strip()),100)
        self.assertEqual(frame.splitlines()[34].strip(),'─'*116)

    def test_padded_renderer_cells_do_not_accept_foreign_session_route_or_stale_settings(self):
        for kwargs in ({'session':str(uuid.uuid4())},{'route':str(uuid.uuid4())},{'stale':True}):
            with self.subTest(kwargs=kwargs),self.assertRaises(AssertionError):
                launcher.settings_selection(self.settings_terminal(**kwargs),self.item['id'],launcher.native_route_id(self.config))

    def test_legacy_route_hash_vectors_match_existing_local_and_public_client_construction(self):
        # Independent fixed vectors from connections.rs::LegacyRoute::id:
        # domain, little-endian directory length+bytes, optional-access marker
        # and bytes, first16 SHA256 bytes, RFC variant and UUID version8.
        local={'native_route':{'mode':'local','directory':'/home/vessel/.local/state/voyage/vessel'}}
        public={'native_route':{'mode':'public'},'access_file':'/private/owner-access.json'}
        self.assertEqual(launcher.native_route_id(local),'cf5ae4cd-6b80-8d6c-8e45-325b21a9fb4c')
        self.assertEqual(launcher.native_route_id(public),'32904b20-3c5b-8793-961a-6ff11e5d16e6')


if __name__=='__main__':unittest.main()
