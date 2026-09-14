#!/usr/bin/env python3
"""Actual saved-answer copy and changed-file selection through supervised offline Helm."""
import argparse, base64, importlib.util, json, pathlib, subprocess, sys
ROOT=pathlib.Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('result_actions_fixture',ROOT/'helm/tests/ux272_journeys.py')
u=importlib.util.module_from_spec(spec);sys.modules[spec.name]=u;spec.loader.exec_module(u)

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--bin-dir',type=pathlib.Path,required=True);parser.add_argument('--output',type=pathlib.Path,required=True);a=parser.parse_args()
    bins=a.bin_dir.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
    f=u.Fixture(bins);j=u.Journey(f,bins,out)
    try:
        f.start();f.seed_account();f.unrestricted=True
        def git(*args):subprocess.run(['git',*args],cwd=f.workspace,check=True,capture_output=True)
        git('init','-q');(f.workspace/'a file.txt').write_text('original\n');(f.workspace/'other.txt').write_text('other\n');git('add','.')
        git('-c','user.name=Synthetic','-c','user.email=synthetic@example.invalid','commit','-qm','baseline')
        (f.workspace/'a file.txt').write_text('selected-file-marker\n');(f.workspace/'other.txt').write_text('other-file-marker\n')
        before={p.name:u.sha(p) for p in f.workspace.iterdir() if p.is_file()}
        sid=f.session();f.command(sid,f.submit(sid,'UXANSWER'));snap=j.finish(sid)
        ui=j.connect();ui.until(lambda s:'[Copy]' in s.text(),'answer action row')
        ui.send(b'\x1bm'); u.settle(ui); (out/'focus.txt').write_text(ui.screen.text()); ui.send(b'\x1bc')  # visible saved answer focus, then explicit Copy
        u.settle(ui); (out/'after-copy.txt').write_text(ui.screen.text()); ui.until(lambda s:'clipboard' in s.text().lower(),'selected answer disclosure')
        (out/'copy-review.txt').write_text(ui.screen.text());ui.send(b'\r')
        def copied():ui.pump();return b'\x1b]52;' in pathlib.Path(ui.log.name).read_bytes()
        u.recovery.wait_for(copied)
        data=pathlib.Path(ui.log.name).read_bytes().split(b'\x1b]52;',1)[1].split(b';',1)[1].split(b'\x07',1)[0].split(b'\x1b\\',1)[0]
        assert base64.b64decode(data).decode()==next(m['content'] for m in reversed(snap['messages']) if m['role']=='assistant')
        u.command_text(ui,'/diff');ui.until(lambda s:'Changes and files' in s.text(),'inspection')
        ui.send(b'\r')
        ui.until(lambda s:'a file.txt' in s.text() and 'Select file:' in s.text(),'selectable status paths',timeout=45)
        (out/'changed-paths.txt').write_text(ui.screen.text());ui.send(b'\t\r')
        ui.until(lambda s:'selected-file-marker' in s.text(),'selected file content',timeout=45)
        (out/'file-read.txt').write_text(ui.screen.text());ui.send(b'u')
        ui.until(lambda s:'+selected-file-marker' in s.text(),'literal file diff',timeout=45)
        text=ui.screen.text();assert '+other-file-marker' not in text,text
        (out/'selected-diff.txt').write_text(text)
        ui.send(b'a')
        ui.until(lambda s:'stdout:' in s.text() and ' M ' in s.text() and 'a file.txt' in s.text(),'refresh file list',timeout=45)
        u.settle(ui)
        lines=ui.screen.text().splitlines()
        row=next(i for i,line in enumerate(lines) if 'a file.txt' in line and ' M ' not in line and 'Path:' not in line and 'diff ' not in line)
        col=lines[row].index('a file.txt')
        ui.send(f'\x1b[<0;{col+1};{row+1}M'.encode())
        ui.until(lambda s:'selected-file-marker' in s.text(),'clicked file read',timeout=45)
        (out/'clicked-file.txt').write_text(ui.screen.text())
        assert before=={p.name:u.sha(p) for p in f.workspace.iterdir() if p.is_file()}
        assert len(f.provider.bodies)==1,'inspection invoked provider'
        (out/'result.json').write_text(json.dumps({'status':'passed','checks':['saved canonical answer copy','select changed path with space','read exact file','mouse click path reads exact file','diff only selected path','no workspace mutation','no extra provider request']},indent=2))
    finally:
        errors=j.close()
        if errors:raise RuntimeError(errors)
if __name__=='__main__':main()
