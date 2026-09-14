#!/usr/bin/env python3
"""Slow catalogue must not leave chooser loading forever; explicit retry only."""
import argparse, importlib.util, json, pathlib, sys, time
ROOT=pathlib.Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('loading_model_fixture',ROOT/'helm/tests/ux272_journeys.py')
u=importlib.util.module_from_spec(spec);sys.modules[spec.name]=u;spec.loader.exec_module(u)
class Slow(u.Provider):
    def do_GET(self):
        self.server.catalog_requests+=1
        # Hold the first actual model catalogue request past the UI deadline.
        self.server.release.wait(60)
        data=json.dumps({'data':[{'id':'fixture-model'},{'id':'retry-model'}]}).encode()
        try:
            self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
        except (BrokenPipeError,ConnectionResetError):pass

def main():
    p=argparse.ArgumentParser();p.add_argument('--bin-dir',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args()
    bins=a.bin_dir.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
    u.Provider=Slow;f=u.Fixture(bins);f.provider.catalog_requests=0;j=u.Journey(f,bins,out)
    try:
        f.start();f.seed_account();sid=f.session();ui=j.connect();u.paste(ui,'keep loading draft');u.settle(ui)
        def click(label):
            u.settle(ui);lines=ui.screen.text().splitlines();row=next(i for i,line in enumerate(lines) if label in line);col=lines[row].index(label)
            ui.send(f'\x1b[<0;{col+1};{row+1}M\x1b[<0;{col+1};{row+1}m'.encode());u.settle(ui)
        click('Model:');ui.until(lambda s:'Choose a model' in s.text(),'chooser opens')
        ui.until(lambda s:'loading…' in s.text(),'loading visibly starts',timeout=5)
        started=time.monotonic();ui.until(lambda s:'loading…' not in s.text() and ('Retry' in s.text()),'loading settles',timeout=17)
        elapsed=time.monotonic()-started;(out/'timeout.txt').write_text(ui.screen.text())
        assert 5<elapsed<17;assert 'fixture-model' in ui.screen.text();assert not f.snapshot(sid)['messages']
        f.provider.release.set();click('[Retry]')
        ui.until(lambda s:'loading…' not in s.text() and 'retry-model' in s.text(),'retry loads catalogue',timeout=17)
        (out/'retry.txt').write_text(ui.screen.text());click('[Cancel]');assert 'keep loading draft' in ui.screen.text();assert not f.snapshot(sid)['messages']
        assert len(f.provider.bodies)==0
        assert f.provider.catalog_requests>=1,'fixture did not exercise actual catalogue HTTP'
        (out/'result.json').write_text(json.dumps({'status':'passed','first_settle_seconds':elapsed,'catalogue_http_requests':f.provider.catalog_requests,'checks':['bounded loading','current model retained','explicit retry','cancel retains draft','no prompt or inference']},indent=2))
    finally:
        errors=j.close()
        if errors:raise RuntimeError(errors)
if __name__=='__main__':main()
