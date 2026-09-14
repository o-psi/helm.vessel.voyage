#!/usr/bin/env python3
"""Direct chooser: select without apply, inline advanced, explicit Use model."""
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
        u.paste(ui,'retained chooser draft');u.settle(ui)
        def point(label):
            u.settle(ui);lines=ui.screen.text().splitlines();matches=[(row,line.index(label)) for row,line in enumerate(lines) if label in line]
            assert matches,(label,ui.screen.text());return matches[0]
        def click(label):
            row,col=point(label);ui.send(f'\x1b[<0;{col+1};{row+1}M\x1b[<0;{col+1};{row+1}m'.encode());u.settle(ui)
        for width,height in [(120,40),(40,18)]:
            ui.resize(width,height);u.settle(ui);before=f.snapshot(sid)
            click('Model:');ui.until(lambda s:'Choose a model' in s.text(),'direct model list')
            assert 'Model options' not in ui.screen.text();assert 'Thinking:' not in ui.screen.text()
            u.settle(ui);lines=ui.screen.text().splitlines();row=next(i for i,l in enumerate(lines) if '┌ Choose a model' in l);col=lines[row].index('┌');w=min(width-2,86);h=min(height-2,24)
            assert abs(col-(width-col-w))<=1;assert abs(row-(height-row-h))<=1
            (out/f'chooser-{width}.txt').write_text(ui.screen.text())
            click('fixture-model (current)');assert 'Choose a model' in ui.screen.text();assert f.snapshot(sid)['revision']==before['revision']
            click('Advanced options');assert 'Thinking:' in ui.screen.text() and 'Service:' in ui.screen.text()
            (out/f'advanced-{width}.txt').write_text(ui.screen.text())
            click('[Cancel]');assert 'retained chooser' in ui.screen.text();assert f.snapshot(sid)['revision']==before['revision']
        ui.resize(120,40);u.settle(ui);click('Model:');ui.until(lambda s:'Choose a model' in s.text(),'chooser')
        click('Account:');ui.until(lambda s:'Select account' in s.text(),'inline account list')
        assert 'Settings for next run' not in ui.screen.text()
        ui.send(b'\x1b');ui.until(lambda s:'Choose a model' in s.text(),'return to chooser')
        click('fixture-model (current)');click('[Use model]')
        ui.until(lambda s:'Model: fixture-model' in s.text() and 'Choose a model' not in s.text(),'explicit application')
        assert 'retained chooser' in ui.screen.text();assert not f.snapshot(sid)['messages'];assert len(f.provider.bodies)==0
        (out/'result.json').write_text(json.dumps({'status':'passed','dimensions':[[120,40],[40,18]],'checks':['direct list','centered','row selection unapplied','collapsed and inline advanced','cancel preserves settings','inline account list and return','explicit Use model','draft retained','no prompt/provider inference']},indent=2))
    finally:
        errors=j.close()
        if errors:raise RuntimeError(errors)
if __name__=='__main__':main()
