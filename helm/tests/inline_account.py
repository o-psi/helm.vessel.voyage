"""Account/model selection stays in one chooser; old account editor is absent."""
import argparse,importlib.util,json,pathlib,subprocess,sys,uuid
ROOT=pathlib.Path(__file__).resolve().parents[2]
s=importlib.util.spec_from_file_location('inline_account_fixture',ROOT/'helm/tests/ux272_journeys.py');u=importlib.util.module_from_spec(s);sys.modules[s.name]=u;s.loader.exec_module(u)
class Models(u.Provider):
    def do_GET(self):
        data=json.dumps({'data':[{'id':'fixture-model'},{'id':'other-model'}]}).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
def main():
    p=argparse.ArgumentParser();p.add_argument('--bin-dir',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args();bins=a.bin_dir.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
    u.Provider=Models;f=u.Fixture(bins);j=u.Journey(f,bins,out)
    try:
        f.start();f.seed_account();first=f.binding.copy();r=subprocess.run([str(bins/'vessel'),'auth','accounts','add','--connection',first['connection_id'],'--account','second-account','--env','PROVIDER_FIXTURE_KEY'],env=f.env,cwd=f.workspace,check=True,capture_output=True,text=True);second=json.loads(r.stdout);sid=f.session();ui=j.connect();u.paste(ui,'retain inline account draft');u.settle(ui)
        def click(label):
            u.settle(ui);lines=ui.screen.text().splitlines();row=next(i for i,l in enumerate(lines) if label in l);col=lines[row].index(label);ui.send(f'\x1b[<0;{col+1};{row+1}M'.encode());u.settle(ui)
        before=f.snapshot(sid)
        for width,height in [(120,40),(40,18)]:
            ui.resize(width,height);u.settle(ui);click('Model:');ui.until(lambda s:'Choose a model' in s.text(),'chooser')
            click('Account:');ui.until(lambda s:'second-account' in s.text() and 'Loading accounts' not in s.text(),'inline account choices')
            text=ui.screen.text();assert 'Choose a model' in text and 'Settings for next run' not in text and 'Choose account' not in text,text
            (out/f'accounts-{width}.txt').write_text(text);click('second-account');ui.until(lambda s:'Account: second-account' in s.text() and 'other-model' in s.text(),'second account models')
            assert f.snapshot(sid)['inference']['account']==first;assert f.snapshot(sid)['revision']==before['revision'];assert 'Settings for next run' not in ui.screen.text()
            click('[Cancel]');assert 'retain inline' in ui.screen.text();assert f.snapshot(sid)['inference']['account']==first
        ui.resize(120,40);u.settle(ui);click('Model:');click('Account:');ui.until(lambda s:'second-account' in s.text() and 'Loading accounts' not in s.text(),'account list')
        click('second-account');ui.until(lambda s:'other-model' in s.text(),'models');click('other-model');assert f.snapshot(sid)['inference']['account']==first
        click('[Use model]');u.recovery.wait_for(lambda:f.snapshot(sid)['inference']['account']['account_id']==second['id']);snap=f.snapshot(sid)
        assert snap['inference']['model']=='other-model';assert not snap['messages'];assert len(f.provider.bodies)==0;assert f.request({'op':'account_defaults','workspace':str(f.workspace)})['account']==first
        ui.until(lambda s:'Model: other-model' in s.text(),'new model visible');assert 'retain inline' in ui.screen.text()
        (out/'applied.txt').write_text(ui.screen.text());(out/'result.json').write_text(json.dumps({'status':'passed','checks':['account inline at both sizes','old form absent','cancel preserves original account/model','single explicit combined apply','host default unchanged','draft retained','no prompt/inference']},indent=2))
    finally:
        errors=j.close()
        if errors:raise RuntimeError(errors)
if __name__=='__main__':main()
