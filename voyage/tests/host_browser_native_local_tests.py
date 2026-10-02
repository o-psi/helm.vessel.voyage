"""Prepared private selection/principal contracts, no Native or TLS effects."""
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import unittest
import uuid
import sys
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
        return {'schema':1,'directory':str(directory),'vessel_id':str(uuid.uuid4()),'sessions':sessions}

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


if __name__=='__main__':unittest.main()
