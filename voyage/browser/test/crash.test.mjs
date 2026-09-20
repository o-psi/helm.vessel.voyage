// Linux-only, real sandboxed Chromium crash qualification. No providers/network pages.
// Reproduce: node --test voyage/browser/test/crash.test.mjs
// Raw fixture identities stay in ignored target/browser333-crash; TAP is PID-free.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawn} from 'node:child_process';
import {randomUUID} from 'node:crypto';

const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../../..');
const guardian=path.join(root,'voyage/browser/guardian.py');
const worker=path.join(root,'voyage/browser/worker.mjs');
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const run=randomUUID();
const evidence=path.join(root,'target/browser333-crash',run);
const executable=process.env.CHROMIUM||'/usr/bin/chromium';

async function identity(pid){
  try{
    const stat=await fs.readFile(`/proc/${pid}/stat`,'utf8');
    const f=stat.slice(stat.lastIndexOf(')')+2).split(' ');
    return {pid:Number(pid),state:f[0],ppid:Number(f[1]),pgrp:Number(f[2]),session:Number(f[3]),start:f[19]};
  }catch(e){if(['ENOENT','ESRCH'].includes(e.code))return null;throw e;}
}
const same=(a,b)=>a&&b&&a.pid===b.pid&&a.start===b.start;
async function signal(p,s){
  // Never signal a process group: Chromium launches in independent groups.
  if(same(p,await identity(p.pid))){try{process.kill(p.pid,s);}catch(e){if(e.code!=='ESRCH')throw e;}}
}
async function inventory(token,known){
  const all=[];
  for(const name of await fs.readdir('/proc')){
    if(!/^\d+$/.test(name))continue;
    const p=await identity(name);if(!p)continue;
    let owned=same(known.get(p.pid),p);
    if(!owned){try{owned=(await fs.readFile(`/proc/${name}/environ`)).toString().split('\0').includes(`BROWSER333_CRASH_FIXTURE=${token}`);}catch(e){if(!['ENOENT','ESRCH','EACCES','EPERM'].includes(e.code))throw e;}}
    all.push({...p,owned});
  }
  // Retain descendants even if their environment is scrubbed, then track identity
  // across reparenting. The fixture marker also discovers double-forked crashpad.
  let changed=true;
  while(changed){changed=false;for(const p of all)if(!p.owned&&all.some(a=>a.owned&&a.pid===p.ppid)){p.owned=true;changed=true;}}
  const result=[];
  for(const p of all.filter(p=>p.owned)){
    known.set(p.pid,p);
    let cmd='';try{cmd=(await fs.readFile(`/proc/${p.pid}/cmdline`)).toString().split('\0').filter(Boolean);}catch{}
    const sockets=[];
    try{for(const fd of await fs.readdir(`/proc/${p.pid}/fd`)){try{const link=await fs.readlink(`/proc/${p.pid}/fd/${fd}`);if(/^socket:\[\d+\]$/.test(link))sockets.push(link);}catch{}}}catch{}
    result.push({...p,cmd,sockets});
  }
  return result;
}
async function tree(dir){
  const entries=[];
  async function walk(at){for(const d of await fs.readdir(at,{withFileTypes:true})){const p=path.join(at,d.name);entries.push({path:path.relative(dir,p),type:d.isDirectory()?'directory':d.isSymbolicLink()?'symlink':d.isSocket()?'socket':'file'});if(d.isDirectory())await walk(p);}}
  try{await walk(dir);}catch(e){if(e.code!=='ENOENT')throw e;}return entries;
}
async function snapshot(token,known,tmp,home){
  const processes=await inventory(token,known);
  const inodes=new Set(processes.flatMap(p=>p.sockets.map(s=>s.slice(8,-1))));
  const sockets={};
  for(const name of ['unix','tcp','tcp6','udp','udp6']){
    const lines=(await fs.readFile(`/proc/net/${name}`,'utf8')).split('\n');
    sockets[name]=lines.filter(line=>line.includes(tmp)||line.trim().split(/\s+/).some(v=>inodes.has(v)));
  }
  return {processes,sockets,files:[...await tree(tmp),...(await tree(home)).map(f=>({...f,path:'state/'+f.path}))]};
}
const summarize=s=>({live:s.processes.filter(p=>p.state!=='Z').length,zombies:s.processes.filter(p=>p.state==='Z').length,groups:new Set(s.processes.map(p=>p.pgrp)).size,socketDescriptors:s.processes.reduce((n,p)=>n+p.sockets.length,0),profileDirectories:s.files.filter(f=>f.type==='directory'&&/^playwright_chromiumdev_profile-[^/]+$/.test(f.path)).length,socketFiles:s.files.filter(f=>f.type==='socket').length,workerLock:s.files.some(f=>f.path==='state/worker.lock')});

for(const scenario of ['sigkill','hung-eof','hung-term','shutdown'])test(`actual worker crash cleanup: ${scenario}`,{skip:process.platform!=='linux',timeout:60000},async t=>{
  await fs.mkdir(evidence,{recursive:true,mode:0o700});
  // Keep Chromium singleton Unix socket paths well below sockaddr_un's limit.
  const fixture=await fs.mkdtemp('/tmp/b333-');await fs.chmod(fixture,0o700);
  const tmp=path.join(fixture,'tmp'),home=path.join(fixture,'state');
  await fs.mkdir(tmp,{mode:0o700});await fs.mkdir(home,{mode:0o700});
  const token=randomUUID(),known=new Map(),pending=new Map();
  const child=spawn('/usr/bin/python3',[guardian,process.execPath,worker,tmp],{cwd:root,env:{...process.env,HOME:home,TMPDIR:tmp,BROWSER333_CRASH_FIXTURE:token},stdio:['pipe','pipe','pipe'],detached:true});
  let exit=null,buffer='',stderr='';
  child.on('exit',(code,signal)=>{exit={code,signal};});
  child.stderr.on('data',b=>{stderr=(stderr+b).slice(-32768);});
  child.stdin.on('error',()=>{});
  child.stdout.on('data',b=>{buffer+=b;for(;;){const at=buffer.indexOf('\n');if(at<0)break;const line=buffer.slice(0,at);buffer=buffer.slice(at+1);try{const reply=JSON.parse(line);pending.get(reply.id)?.(reply);}catch{}}});
  const initial=await identity(child.pid);if(initial)known.set(child.pid,initial);
  let status={},before,after,cleanup,summary,setupError,marker;
  // Same executable, but NOT a descendant: process names confer no ownership.
  const sentinel=spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{env:{...process.env},stdio:'ignore'});
  const sentinelIdentity=await identity(sentinel.pid);
  const receiptName=randomUUID()+'.json';
  const receipt=JSON.stringify({id:receiptName.slice(0,-5),state:'dispatched',digest:'0'.repeat(64),time:Date.now()});
  const call=async(op,args={})=>{
    const id=randomUUID();let timer;
    try{
      const reply=await Promise.race([new Promise(resolve=>{pending.set(id,resolve);child.stdin.write(JSON.stringify({id,op,browser:status.browser,epochs:status.epochs,...args})+'\n');}),new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('worker request timeout')),20000);})]);
      assert.equal(reply.ok,true,`worker ${op} failed (${reply.error?.code||'unknown'})`);status=reply.result.status;return reply;
    }finally{clearTimeout(timer);pending.delete(id);}
  };
  try{
    await call('init',{config:{root:home,executable,public_web:false,origins:[],ice_servers:[],width:640,height:480}});
    await call('open');
    // A dispatched external effect cannot be declared reconciled by killing PIDs.
    await fs.writeFile(path.join(home,'receipts',receiptName),receipt,{mode:0o600});
    before=await snapshot(token,known,tmp,home);
    // Chromium may rewrite its process title into one argv entry on this host.
    // Ownership still comes from the observed descendant identities above.
    const browsers=before.processes.filter(p=>Array.isArray(p.cmd)&&/(?:^|\s)--user-data-dir=/.test(p.cmd.join(' '))&&!/(?:^|\s)--type=/.test(p.cmd.join(' ')));
    assert.equal(browsers.length,2,'both task and encoder Chromium must actually be running');
    assert.ok(before.processes.some(p=>p.pgrp!==initial.pgrp),'must observe Chromium outside Node process group');
    const node=before.processes.find(p=>p.ppid===child.pid&&p.cmd.includes(worker));
    assert.ok(node,'guardian must launch actual Node worker');
    if(scenario==='shutdown'){const reply=await call('shutdown');assert.equal(reply.result.value.shutdown,true,'shutdown reply must traverse guardian before exit');}
    else if(scenario==='sigkill')await signal(node,'SIGKILL');
    else{
      // Stop Node AND every browser descendant. Guardian alone must detect EOF
      // or TERM and clean independent groups, without worker cooperation.
      for(const p of before.processes.filter(p=>p.pid!==child.pid))await signal(p,'SIGSTOP');
      if(scenario==='hung-eof')child.stdin.end();else await signal(initial,'SIGTERM');
    }
    const deadline=Date.now()+12000;while(!exit&&Date.now()<deadline)await sleep(100);
    await sleep(1500);after=await snapshot(token,known,tmp,home);
    marker=JSON.parse(await fs.readFile(path.join(home,'guardian-cleanup.json'),'utf8'));
    assert.equal((await fs.stat(path.join(home,'guardian-cleanup.json'))).mode&0o777,0o600);
    await assert.rejects(fs.stat(tmp),{code:'ENOENT'},'guardian must remove entire scratch directory');
    assert.equal(await fs.readFile(path.join(home,'receipts',receiptName),'utf8'),receipt,'uncertain receipt must remain byte-identical');
    assert.ok(same(sentinelIdentity,await identity(sentinel.pid)),'guardian killed unrelated process');
    summary={scenario,marker,workerExited:!!exit,exit,before:summarize(before),after:summarize(after)};
  }catch(e){setupError=e;}
  finally{
    await signal(sentinelIdentity,'SIGKILL');
    await new Promise(resolve=>sentinel.exitCode!==null||sentinel.signalCode!==null?resolve():sentinel.once('exit',resolve));
    // Preserve pre-cleanup obligations even when teardown succeeds. Identity-
    // checked, fixture-only kill; do not use pkill, parent group, or name matching.
    for(let pass=0;pass<4;pass++){
      const owned=await inventory(token,known);
      for(const p of owned.filter(p=>p.state!=='Z'))await signal(p,'SIGKILL');
      await sleep(250);
    }
    cleanup=await snapshot(token,known,tmp,home);
    const remaining=summarize(cleanup);
    const uncertain=remaining.live>0||remaining.zombies>0;
    await fs.writeFile(path.join(evidence,`${scenario}.json`),JSON.stringify({scenario,tmp,token,before,after,cleanup,summary,marker,exit,stderr,setupError:setupError?.message,cleanupUncertain:uncertain},null,2),{mode:0o600});
    // Retain profiles on uncertainty, otherwise archive inventory then remove only
    // the exact mkdtemp fixture. Zombies are separately reported, not live browsers.
    if(!uncertain)await fs.rm(fixture,{recursive:true,force:true});
    t.diagnostic(JSON.stringify({...(summary||{scenario,setupFailed:true}),fixtureCleanup:remaining,fixtureRetained:uncertain}));
    assert.equal(uncertain,false,'fixture cleanup uncertain: exact private fixture retained in evidence');
  }
  if(setupError)throw setupError;
  assert.ok(summary.workerExited,'worker did not exit within bounded deadline');
  assert.equal(summary.after.live,0,'crash left live fixture descendants: outstanding resource obligation');
  assert.equal(summary.after.socketFiles,0,'crash left socket filesystem entries');
  assert.equal(summary.after.profileDirectories,0,'crash left private Chromium profiles');
  assert.equal(summary.after.zombies,0,'guardian must reap every owned descendant');
  assert.equal(summary.after.workerLock,scenario!=='shutdown','only cooperative worker shutdown may release its lock');
  assert.equal(marker.observed,true);
  assert.equal(marker.cleanup_complete,true);
  assert.equal(marker.external_actions_reconciled,false);
  assert.equal(marker.forced,scenario.startsWith('hung-'));
  assert.equal(marker.worker_status,scenario==='shutdown'?0:-9);
});
