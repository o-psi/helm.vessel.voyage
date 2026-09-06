#!/usr/bin/env python3
"""Offline enrollment discovery/revocation/audit with real local binaries, no providers."""
import json
import base64
import http.client
import sqlite3
import os
import pathlib
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid

ROOT=pathlib.Path(__file__).resolve().parents[2]
HELM=pathlib.Path(os.environ.get('HELM_BIN',ROOT/'target/release/helm')).resolve()
VESSEL=pathlib.Path(os.environ.get('VESSEL_BIN',ROOT/'target/release/vessel')).resolve()
TOKEN='inventory-operator-fixture-token-at-least-32-bytes'


def main():
    with tempfile.TemporaryDirectory(prefix='voyage-inventory-') as temporary:
        root=pathlib.Path(temporary)
        env=dict(os.environ,HOME=str(root/'home'),XDG_DATA_HOME=str(root/'data'),XDG_CONFIG_HOME=str(root/'config'),VESSEL_OPERATOR_TOKEN=TOKEN,RUST_LOG='trace')
        invalid=root/'invalid.toml';invalid.write_text('invalid provider [')
        with socket.socket() as sock:
            sock.bind(('127.0.0.1',0));address=f'127.0.0.1:{sock.getsockname()[1]}'
        origin='http://'+address
        children=[];secrets=[TOKEN];log=tempfile.TemporaryFile()
        def safe(value):
            for secret in secrets:assert secret not in value,'credential escaped inspection'
        def request(path,body=None,expected=200,auth=True,headers=None):
            fields=dict(headers or {})
            if auth:fields['Authorization']='Bearer '+TOKEN
            if body is not None:fields.update({'Content-Type':'application/json','x-voyage-request':'2'})
            req=urllib.request.Request(origin+path,data=None if body is None else json.dumps(body).encode(),headers=fields)
            try:response=urllib.request.urlopen(req,timeout=8)
            except urllib.error.HTTPError as error:response=error
            with response:
                value=response.read().decode();assert response.status==expected,(path,response.status,expected)
                if path.startswith('/v2/enrollment/machines') or path.startswith('/v2/enrollment/audit'):
                    assert response.headers.get('Cache-Control')=='no-store'
            safe(value)
            return json.loads(value) if value and expected==200 else None
        def cli(identity,*args,key=None,expected=0):
            result=subprocess.run([str(HELM),'--config',str(invalid),'attachment','--directory',str(identity),'--allow-insecure-loopback',*map(str,args)],input=key,text=True,capture_output=True,env=env,timeout=12)
            safe(result.stdout+result.stderr);assert result.returncode==expected,(args,result.returncode,result.stderr)
            return json.loads(result.stdout) if result.stdout else None
        def enroll(identity):
            req=urllib.request.Request(origin+'/v2/enrollment/invitations',data=b'{"ttl_ms":60000}',headers={'Authorization':'Bearer '+TOKEN,'Content-Type':'application/json','x-voyage-request':'2'})
            with urllib.request.urlopen(req,timeout=5) as response:invitation=json.load(response)
            secrets.append(invitation['key'])
            return cli(identity,'--origin',origin,'enroll','--invitation-id',invitation['id'],'--invitation-key-stdin',key=invitation['key']+'\n')
        def server():
            p=subprocess.Popen([str(VESSEL),'--bind',address,'--database',str(root/'vessel.db'),'--attachment-directory',str(root/'authority'),'--public-origin',origin+'/','--allow-insecure-loopback'],env=env,stdout=log,stderr=log);children.append(p)
            deadline=time.monotonic()+10
            while True:
                assert p.poll() is None,'startup failed'
                try:request('/ready',auth=False);return p
                except (OSError,urllib.error.URLError):
                    assert time.monotonic()<deadline,'startup timeout';time.sleep(.03)
        try:
            active=server()
            first=enroll(root/'lost');second=enroll(root/'retained')
            # The lost device has no live socket or accessible command process.
            assert request('/v1/diagnostics')['connections']==[]
            page=request('/v2/enrollment/machines',{'limit':1})
            assert len(page['machines'])==1 and page['next']
            other=request('/v2/enrollment/machines',{'limit':1,'cursor':page['next']})
            records=page['machines']+other['machines']
            assert {r['machine_id'] for r in records}=={first['machine_id'],second['machine_id']}
            for item in records:assert set(item)=={'machine_id','epoch','revoked'}
            for route in ['/v2/enrollment/machines','/v2/enrollment/audit']:
                request(route,{'limit':1},expected=401,auth=False)
                request(route,{'limit':1},expected=401,auth=False,headers={'Authorization':'Bearer foreign-owner-credential'})
                request(route,{'limit':1},expected=403,headers={'Origin':'https://foreign.invalid'})
                request(route,{'limit':1},expected=403,headers={'Sec-Fetch-Site':'cross-site'})
                request(route,{'limit':0},expected=400)
                request(route,{'limit':129},expected=400)
                request(route,{'limit':1,'secret':'canary'},expected=400)
                request(route+'?limit=1',{'limit':1},expected=400)
                request(route,{'cursor':'A'*5000},expected=413)
                basic=base64.b64encode(('operator:'+TOKEN).encode()).decode()
                request(route,{'limit':1},auth=False,headers={'Authorization':'Basic '+basic,'Origin':origin})
                for duplicate,value in [('Authorization','Bearer '+TOKEN),('Origin',origin),('x-voyage-request','2'),('sec-fetch-site','same-origin')]:
                    connection=http.client.HTTPConnection(address,timeout=5);connection.putrequest('POST',route)
                    for key,val in [('Authorization','Bearer '+TOKEN),('Origin',origin),('x-voyage-request','2'),('sec-fetch-site','same-origin'),('Content-Type','application/json'),(duplicate,value)]:connection.putheader(key,val)
                    connection.putheader('Content-Length','11');connection.endheaders(b'{"limit":1}')
                    response=connection.getresponse();assert response.status in (401,403),(duplicate,response.status);safe(response.read().decode());connection.close()
            # Authenticate before reading a slow body; authorized reads share bounded operator slots.
            def slow_body(authenticated):
                peer=socket.create_connection(('127.0.0.1',int(address.rsplit(':',1)[1])),timeout=3)
                auth_header=('Authorization: Bearer '+TOKEN+'\r\n') if authenticated else ''
                peer.sendall(('POST /v2/enrollment/machines HTTP/1.1\r\nHost: '+address+'\r\n'+auth_header+
                    'Content-Type: application/json\r\nx-voyage-request: 2\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{').encode())
                return peer
            with slow_body(False) as peer:
                assert b' 401 ' in peer.recv(4096).split(b'\r\n',1)[0]
            stalled=[slow_body(True) for _ in range(4)]
            try:
                time.sleep(.05)
                request('/v2/enrollment/machines',{'limit':1},expected=503)
                started=time.monotonic()
                stalled[0].settimeout(12)
                assert b' 503 ' in stalled[0].recv(4096).split(b'\r\n',1)[0]
                assert time.monotonic()-started<12
            finally:
                for peer in stalled:peer.close()
            time.sleep(.05)
            request('/v2/enrollment/audit',{'limit':1,'cursor':page['next']},expected=403)
            request('/v2/enrollment/machines',{'limit':2,'cursor':page['next']},expected=403)
            malformed=page['next'][:5]+('A' if page['next'][5]!='A' else 'B')+page['next'][6:]
            request('/v2/enrollment/machines',{'limit':1,'cursor':malformed},expected=403)
            snapshot=request('/v2/enrollment/audit',{'limit':1})
            assert snapshot['through']==4 and snapshot['retention']=='all_retained'
            rotated=cli(root/'retained','rotate');assert rotated['epoch']==2
            request('/v2/enrollment/machines',{'limit':1,'cursor':page['next']},expected=409)
            request('/v2/enrollment/revoke',{'machine_id':second['machine_id'],'expected_epoch':1,'transaction_id':str(uuid.uuid4())},expected=403)
            events=list(snapshot['events']);cursor=snapshot['next']
            while cursor:
                part=request('/v2/enrollment/audit',{'limit':1,'cursor':cursor});assert part['through']==4
                events+=part['events'];cursor=part['next']
            assert [event['sequence'] for event in events]==[1,2,3,4]
            lost=next(r for r in records if r['machine_id']==first['machine_id'])
            revoke={'machine_id':lost['machine_id'],'expected_epoch':lost['epoch'],'transaction_id':str(uuid.uuid4())}
            receipt=request('/v2/enrollment/revoke',revoke)
            assert receipt['revoked'] and request('/v2/enrollment/revoke',revoke)==receipt
            cli(root/'lost','connect',expected=1)
            audit=request('/v2/enrollment/audit',{'limit':100})
            assert any(event['kind']=='revoked' and event['machine_id']==lost['machine_id'] for event in audit['events'])
            assert [event['sequence'] for event in audit['events']]==list(range(1,audit['through']+1))
            assert len([event for event in audit['events'] if event['kind']=='revoked'])==1
            tail=request('/v2/enrollment/audit',{'limit':100,'after':4});assert [e['kind'] for e in tail['events']]==['rotated','revoked']
            # Read pages are not writes to authority or its audit.
            database=root/'authority/enrollment.sqlite3'
            before=database.read_bytes()
            resumable=request('/v2/enrollment/audit',{'limit':1})
            request('/v2/enrollment/machines',{'limit':100})
            assert database.read_bytes()==before
            with sqlite3.connect(database) as held:
                held.execute('BEGIN EXCLUSIVE')
                request('/v2/enrollment/machines',{'limit':1},expected=503)
                held.rollback()
            assert database.read_bytes()==before
            active.send_signal(signal.SIGTERM);active.wait(timeout=8)
            active=server()
            continued=request('/v2/enrollment/audit',{'limit':1,'cursor':resumable['next']})
            assert continued['events'][0]['sequence']==2 and continued['through']==resumable['through']
            final=request('/v2/enrollment/machines',{'limit':100})
            assert next(r for r in final['machines'] if r['machine_id']==lost['machine_id'])['revoked']
            cli(root/'lost','connect',expected=1)
            # Distinct authority, same operator token: a copied cursor has no authority.
            active.send_signal(signal.SIGTERM);active.wait(timeout=8)
            original_authority=root/'authority';original_authority.rename(root/'saved-authority')
            active=server()
            request('/v2/enrollment/audit',{'limit':1,'cursor':resumable['next']},expected=403)
            assert request('/v2/enrollment/machines',{'limit':100})['machines']==[]
            active.send_signal(signal.SIGTERM);active.wait(timeout=8)
            log.seek(0);safe(log.read().decode(errors='replace'))
        finally:
            for p in reversed(children):
                if p.poll() is None:p.kill()
                p.wait(timeout=5)
            log.close()
    print('enrollment inventory: offline discovery, exact-epoch revoke and audit passed')


if __name__=='__main__':main()
