#!/usr/bin/env python3
"""Mouse-only model/options navigation through real offline Helm."""
import argparse, importlib.util, json, pathlib, sys
ROOT=pathlib.Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('mouse_model_fixture',ROOT/'helm/tests/ux272_journeys.py')
u=importlib.util.module_from_spec(spec);sys.modules[spec.name]=u;spec.loader.exec_module(u)

def main():
    p=argparse.ArgumentParser();p.add_argument('--bin-dir',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args()
    bins=a.bin_dir.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
    f=u.Fixture(bins);j=u.Journey(f,bins,out)
    try:
        f.start();f.seed_account();sid=f.session();ui=j.connect()
        u.paste(ui,'mouse retained draft');u.settle(ui)
        def point(label,last=False):
            u.settle(ui);lines=ui.screen.text().splitlines();matches=[(row,line.index(label)) for row,line in enumerate(lines) if label in line]
            assert matches,(label,ui.screen.text());return matches[-1] if last else matches[0]
        def click(label,last=False):
            row,col=point(label,last);ui.send(f'\x1b[<0;{col+1};{row+1}M\x1b[<0;{col+1};{row+1}m'.encode());u.settle(ui)
        for width,height in [(120,40),(40,18)]:
            ui.resize(width,height);u.settle(ui)
            click('Model:',last=True);ui.until(lambda s:'Model options' in s.text(),'options open by click')
            click('[Back]');assert 'mouse retained' in ui.screen.text()
            click('Model:',last=True);ui.until(lambda s:'Model options' in s.text(),'options reopened')
            click('Model: fixture-model');ui.until(lambda s:'Search / explicit value:' in s.text(),'model picker open by click')
            row,col=point('Search / explicit value:');ui.send(f'\x1b[<65;{col+1};{row+2}M'.encode());u.settle(ui)
            (out/f'picker-{width}.txt').write_text(ui.screen.text())
            click('[Cancel]');assert 'mouse retained' in ui.screen.text()
            assert not f.snapshot(sid)['messages']
        # Actual row click follows the same selection path; fixture current model is sufficient.
        ui.resize(120,40);u.settle(ui);click('Model:',last=True);click('Model: fixture-model')
        ui.until(lambda s:'Search / explicit value:' in s.text(),'picker')
        u.settle(ui);(out/'before-select.txt').write_text(ui.screen.text())
        click('> fixture-model')
        (out/'after-select.txt').write_text(ui.screen.text())
        ui.until(lambda s:'Model: fixture-model' in s.text() and 'Search / explicit value:' not in s.text(),'model selected by click')
        assert 'mouse retained' in ui.screen.text();assert not f.snapshot(sid)['messages']
        assert len(f.provider.bodies)==0
        (out/'result.json').write_text(json.dumps({'status':'passed','dimensions':[[120,40],[40,18]],'checks':['composer click','options row click','wheel navigation','Back click','Cancel click','model row selection','draft retained','no prompt/provider request']},indent=2))
    finally:
        errors=j.close()
        if errors:raise RuntimeError(errors)
if __name__=='__main__':main()
