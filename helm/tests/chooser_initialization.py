"""Unconfigured first-use and disconnected account reads stay in the new chooser."""
import argparse,importlib.util,json,pathlib,sys
ROOT=pathlib.Path(__file__).resolve().parents[2]
sp=importlib.util.spec_from_file_location('chooser_init_fixture',ROOT/'helm/tests/ux272_journeys.py');u=importlib.util.module_from_spec(sp);sys.modules[sp.name]=u;sp.loader.exec_module(u)
def main():
    p=argparse.ArgumentParser();p.add_argument('--bin-dir',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);a=p.parse_args();bins=a.bin_dir.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
    f=u.Fixture(bins);j=u.Journey(f,bins,out)
    try:
        f.start();ui=j.connect();ui.send(b'\x0e')
        ui.until(lambda s:'Choose a model' in s.text() and 'No accounts available' in s.text(),'unconfigured inline chooser',timeout=15)
        def assert_new():
            text=ui.screen.text();assert 'Choose account' not in text and 'Settings for next run' not in text,text
            assert 'Choose a model' in text and '[Retry]' in text,text
        assert_new();(out/'no-account.txt').write_text(ui.screen.text())
        ui.send(b'\x1b');u.settle(ui);ui.send(b'\x1b');u.settle(ui);u.paste(ui,'retain first task');ui.send(b'\r')
        ui.until(lambda s:'Choose a model' in s.text() and 'No accounts available' in s.text(),'first send inline setup');assert_new();assert f.request({'op':'catalogue'})==[]
        # Stop only this synthetic supervisor; retry observes refusal, never opens old private list.
        f.supervisor.terminate();f.supervisor.wait(timeout=10);f.supervisor=None
        def click(label):
            u.settle(ui);lines=ui.screen.text().splitlines();r=next(i for i,l in enumerate(lines) if label in l);c=lines[r].index(label);ui.send(f'\x1b[<0;{c+1};{r+1}M'.encode());u.settle(ui)
        click('[Retry]');ui.until(lambda s:'Account list unavailable' in s.text() or 'Account list timed out' in s.text(),'account connection failure',timeout=15);assert_new();(out/'disconnect.txt').write_text(ui.screen.text())
        f.start();click('[Retry]');ui.until(lambda s:'No accounts available' in s.text(),'reconnected account list',timeout=15);assert_new()
        click('[Cancel]');assert 'retain first task' in ui.screen.text();assert f.request({'op':'catalogue'})==[];assert len(f.provider.bodies)==0
        (out/'result.json').write_text(json.dumps({'status':'passed','checks':['unconfigured init new chooser','first-send setup new chooser','connection failure new chooser','retry after restart','old Choose account absent','draft retained','no implicit session or inference']},indent=2))
    finally:
        errors=j.close()
        if errors:raise RuntimeError(errors)
if __name__=='__main__':main()
