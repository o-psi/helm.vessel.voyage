#!/usr/bin/env python3
"""Two accounts: second is host default; chooser must resolve/display/select it."""
import argparse,importlib.util,json,pathlib,subprocess,sys,uuid
ROOT=pathlib.Path(__file__).resolve().parents[2]
sp=importlib.util.spec_from_file_location('account_default_fixture',ROOT/'helm/tests/ux272_journeys.py');u=importlib.util.module_from_spec(sp);sys.modules[sp.name]=u;sp.loader.exec_module(u)
class Models(u.Provider):
    def do_GET(self):
        data=json.dumps({'data':[{'id':'fixture-model'},{'id':'other-model'}]}).encode()
        self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)

def main():
    p=argparse.ArgumentParser();p.add_argument('--bin-dir',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args();bins=a.bin_dir.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
    u.Provider=Models;f=u.Fixture(bins);j=u.Journey(f,bins,out)
    try:
        f.start();f.seed_account();first=f.binding.copy()
        run=subprocess.run([str(bins/'vessel'),'auth','accounts','add','--connection',first['connection_id'],'--account','zz-default','--env','PROVIDER_FIXTURE_KEY'],env=f.env,cwd=f.workspace,capture_output=True,text=True,check=True)
        second=json.loads(run.stdout);binding={**first,'account_id':second['id'],'identity_generation':second['identity_generation']}
        catalogue=f.request({'op':'accounts','workspace':str(f.workspace),'transport':None})
        f.request({'op':'account_set_default','workspace':str(f.workspace),'command_id':str(uuid.uuid4()),'account':binding,'expected_revision':catalogue['default_revision']})
        order=f.request({'op':'accounts','workspace':str(f.workspace),'transport':None})['accounts'];assert order[0]['id']==first['account_id'] and order[1]['id']==binding['account_id']
        # Start a new unsent local draft; no provider setup detour should remain.
        ui=j.connect();ui.send(b'\x0e');ui.until(lambda s:'Model:' in s.text() and 'Choose account' not in s.text(),'draft inherits default')
        u.paste(ui,'default draft retained');u.settle(ui)
        def click(label):
            u.settle(ui);lines=ui.screen.text().splitlines();row=next(i for i,l in enumerate(lines) if label in l);col=lines[row].index(label);ui.send(f'\x1b[<0;{col+1};{row+1}M'.encode());u.settle(ui)
        click('Model:');ui.until(lambda s:'Choose a model' in s.text() and 'Account: zz-default' in s.text(),'default label before manual account selection')
        (out/'default-chooser.txt').write_text(ui.screen.text());click('Account:');ui.until(lambda s:'Select account' in s.text() and 'zz-default' in s.text(),'inline account list')
        text=ui.screen.text();assert any('● zz-default' in l for l in text.splitlines()),text
        (out/'default-selected.txt').write_text(text);ui.send(b'\x1b');ui.until(lambda s:'Choose a model' in s.text(),'return chooser');click('[Cancel]');assert 'default draft retained' in ui.screen.text()
        assert f.request({'op':'account_defaults','workspace':str(f.workspace)})['account']==binding
        assert f.request({'op':'catalogue'})==[],'opening draft created voyage';assert len(f.provider.bodies)==0
        # Existing explicitly bound voyage must stay on first account despite host default second.
        sid=f.session()
        # A new observer selects actual existing voyage (draft still belongs to first client).
        other=j.connect();other.until(lambda s:'Model: fixture-model' in s.text(),'explicit voyage')
        lines=other.screen.text().splitlines();row=next(i for i,l in enumerate(lines) if '│Model:' in l);col=lines[row].index('Model:');other.send(f'\x1b[<0;{col+1};{row+1}M'.encode())
        other.until(lambda s:'Account: synthetic' in s.text(),'explicit account retained')
        (out/'explicit-chooser.txt').write_text(other.screen.text());assert f.snapshot(sid)['inference']['account']==first
        (out/'result.json').write_text(json.dumps({'status':'passed','checks':['default second not first','new draft resolves default before models','real account label hydrated','default row preselected','explicit voyage binding wins','no host default mutation','no inference or unwanted creation']},indent=2))
    finally:
        errors=j.close()
        if errors:raise RuntimeError(errors)
if __name__=='__main__':main()
