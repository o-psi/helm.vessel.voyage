// Scoped cleanup fault/overlap evidence. Never embedded or enabled in production.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import {fileURLToPath} from 'node:url';
const here=path.dirname(fileURLToPath(import.meta.url));
process.umask(0o077);
const evidence=await fs.mkdtemp(path.join(process.env.HELM_BROWSER_PROBE_ROOT||os.tmpdir(),'helm-browser-cleanup-'));
const source=await fs.readFile(path.join(here,'helper.mjs'),'utf8');
for(const failure of [false,true]){
  const assets=path.join(evidence,failure?'rejected-close':'overlapping-close');await fs.mkdir(assets,{mode:0o700});
  // Only the private fixture copy is instrumented. A real Chromium context is closed
  // before synthesizing rejection, so this check does not intentionally orphan it.
  const injection=`
  const originalClose=context.close.bind(context);let closeCalls=0;
  context.close=async()=>{await fs.writeFile(path.join(root,'probe-close-invocations'),String(++closeCalls),{mode:0o600});await new Promise(r=>setTimeout(r,350));await originalClose();await fs.writeFile(path.join(root,'probe-browser-quiescent'),'observed real Chromium context close',{mode:0o600});${failure?"throw new Error('synthetic close rejection');":""}};
`;
  assert.equal(source.split("  await context.route('**/*'").length,2,'unique fixture insertion point');
  await fs.writeFile(path.join(assets,'helper.mjs'),source.replace("  await context.route('**/*'",injection+"  await context.route('**/*'"),{mode:0o600});
  for(const name of ['security.mjs','index.html','app.js','app.css'])await fs.copyFile(path.join(here,name),path.join(assets,name));
  await fs.symlink(path.join(here,'node_modules'),path.join(assets,'node_modules'),'dir');
  const root=path.join(assets,'session');
  const helper=spawn(process.execPath,[path.join(assets,'helper.mjs')],{stdio:['pipe','pipe','pipe']});
  let buffer='',diagnostics='',counter=0;const pending=new Map(),replies=[];
  helper.stderr.on('data',b=>diagnostics+=b);
  helper.stdout.on('data',b=>{buffer+=b;let i;while((i=buffer.indexOf('\n'))>=0){const r=JSON.parse(buffer.slice(0,i));buffer=buffer.slice(i+1);if(r.event)continue;replies.push(r);pending.get(r.id)?.(r);pending.delete(r.id);}});
  const call=q=>new Promise((resolve,reject)=>{const id=`cleanup-${++counter}`,timer=setTimeout(()=>reject(Error('bounded cleanup fixture timeout')),15000);pending.set(id,r=>{clearTimeout(timer);resolve(r);});helper.stdin.write(JSON.stringify({...q,id})+'\n');});
  try{
    const init=await call({op:'init',session_dir:root});assert.equal(init.ok,true);
    await fs.writeFile(path.join(root,'uploads','private-fixture'),'private staged fixture',{mode:0o600});
    const start=Date.now(),first=call({op:'shutdown'}),second=call({op:'shutdown'});
    const [a,b]=await Promise.all([first,second]);assert.ok(Date.now()-start>=300,'no early closed reply from overlapping shutdown');
    assert.equal(await fs.readFile(path.join(root,'probe-close-invocations'),'utf8'),'1','all callers await one close invocation');
    assert.equal(await fs.readFile(path.join(root,'probe-browser-quiescent'),'utf8'),'observed real Chromium context close');
    if(failure){
      for(const r of [a,b,await call({op:'shutdown'})]){assert.equal(r.ok,false);assert.equal(r.error.code,'cleanup_unresolved');}
      assert.ok(!replies.some(r=>r.result?.closed===true),'no positive cleanup reply after close rejection');
      assert.equal(await fs.readFile(path.join(root,'uploads','private-fixture'),'utf8'),'private staged fixture');
      assert.ok((await fs.stat(path.join(root,'executor.lock'))).isFile(),'failure retains exclusive lock');
      helper.stdin.end();
    }else{
      for(const r of [a,b]){assert.equal(r.ok,true);assert.equal(r.result.closed,true);}
      for(const name of ['executor.lock','uploads','downloads'])await assert.rejects(fs.stat(path.join(root,name)),{code:'ENOENT'});
    }
    if(helper.exitCode===null)await new Promise(r=>helper.once('exit',r));
    assert.equal(helper.exitCode,0);assert.equal(diagnostics,'');
    // Even a clean Node exit does NOT transform the failed-close fixture into success.
    if(failure)assert.ok((await fs.stat(path.join(root,'executor.lock'))).isFile());
  }finally{
    if(helper.exitCode===null){helper.stdin.end();await new Promise((resolve,reject)=>{helper.once('exit',resolve);setTimeout(()=>reject(Error('cleanup fixture helper did not exit; retain evidence and inspect owned processes')),5000).unref();});}
  }
}
console.log('PASS: overlapping shutdown awaits one observed close; close rejection is retained across repeated shutdown; no false closed reply; lock/staging retained despite helper exit. Real Chromium quiesced in both fault fixtures.');
console.log(`Private cleanup evidence: ${evidence}`);
