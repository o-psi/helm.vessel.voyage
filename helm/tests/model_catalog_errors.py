"""Actual helper->Vessel->Helm catalogue error categories without provider bodies."""
import argparse,importlib.util,json,pathlib,sys
ROOT=pathlib.Path(__file__).resolve().parents[2]
sp=importlib.util.spec_from_file_location('model_errors_fixture',ROOT/'helm/tests/ux272_journeys.py');u=importlib.util.module_from_spec(sp);sys.modules[sp.name]=u;sp.loader.exec_module(u)
class Provider(u.Provider):
    def do_GET(self):
        self.server.catalog_calls+=1
        code,body=self.server.catalog_reply
        try:
            self.send_response(code);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        except (BrokenPipeError,ConnectionResetError):pass

def main():
    p=argparse.ArgumentParser();p.add_argument('--bin-dir',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=False)
    rows=[]
    for code,body,label in [(401,b'SECRET-provider-diagnostic','Check your account’s sign-in'),(200,b'{"data":"SECRET-malformed"}','Couldn’t read the model list'),(429,b'SECRET-rate-limit','Too many requests')]:
        u.Provider=Provider;f=u.Fixture(a.bin_dir.resolve());f.provider.catalog_calls=0;f.provider.catalog_reply=(code,body);out=a.output/str(code);out.mkdir();j=u.Journey(f,a.bin_dir.resolve(),out)
        try:
            f.start();f.seed_account();sid=f.session();ui=j.connect();u.paste(ui,'retained diagnostic draft');u.settle(ui)
            lines=ui.screen.text().splitlines();r=next(i for i,l in enumerate(lines) if '│Model:' in l);c=lines[r].index('Model:');ui.send(f'\x1b[<0;{c+1};{r+1}M'.encode())
            ui.until(lambda s:label in s.text(),'safe catalogue failure',timeout=18)
            assert '[Retry]' in ui.screen.text();assert 'SECRET' not in ui.screen.text();assert f.provider.catalog_calls>0;assert len(f.provider.bodies)==0;assert not f.snapshot(sid)['messages']
            (out/'screen.txt').write_text(ui.screen.text());ui.send(b'\x1b');u.settle(ui);assert 'retained diagnostic' in ui.screen.text();rows.append({'http':code,'category_visible':label,'passed':True})
        finally:
            errors=j.close()
            if errors:raise RuntimeError(errors)
    (a.output/'result.json').write_text(json.dumps(rows,indent=2))
if __name__=='__main__':main()
