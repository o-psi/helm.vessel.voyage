import test from 'node:test';
import assert from 'node:assert/strict';
import {randomUUID} from 'node:crypto';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {spawn} from 'node:child_process';
import {Worker} from '../worker.mjs';

test('private acknowledgement waits for old select to settle without changing active page',async()=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'select-fence-'));const w=new Worker();
 const call=(op,args={})=>w.request({id:randomUUID(),op,...w.status(),...args});
 try{
  assert.equal((await call('init',{config:{root,executable:'/usr/bin/chromium',public_web:false,origins:[]}})).ok,true);
  const old={isClosed:()=>false},next={isClosed:()=>false};w.task={close:async()=>{}};w.context={newCDPSession:async()=>({send:async()=>{},detach:async()=>{}})};w.page=old;w.active=randomUUID();w.tabs.set(w.active,old);const tab=randomUUID();w.tabs.set(tab,next);
  const viewer=randomUUID();await call('join',{viewer});
  let entered,release;const started=new Promise(r=>entered=r);let calls=0;
  w.stopCapture=async()=>{if(++calls===1){entered();await new Promise(r=>release=r);}};
  const selecting=call('agent',{action:{kind:'tabs',operation:'select',tab}});await started;
  let acknowledged=false;const privateMode=call('control',{viewer,mode:'private'}).then(r=>{acknowledged=true;return r;});
  for(let i=0;i<100&&w.mode!=='private';i++)await new Promise(r=>setTimeout(r,10));
  assert.equal(w.mode,'private');assert.equal(acknowledged,false);release();
  assert.equal((await selecting).error.code,'control_fenced');assert.equal((await privateMode).ok,true);assert.equal(w.page,old);assert.notEqual(w.active,tab);
 }finally{w.page=null;await w.dispose();await fs.rm(root,{recursive:true,force:true});}
});

test('EOF refuses queued effect before it can launch Chromium',{timeout:15000},async()=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'eof-fence-'));
 const child=spawn(process.execPath,[new URL('../worker.mjs',import.meta.url).pathname],{stdio:['pipe','pipe','pipe']});let out='',err='';child.stdout.on('data',b=>out+=b);child.stderr.on('data',b=>err+=b);
 try{
  child.stdin.write(JSON.stringify({id:randomUUID(),op:'init',config:{root,executable:'/definitely/not/a/browser',public_web:false,origins:[]}})+'\n');
  while(!out.includes('\n'))await new Promise(r=>setTimeout(r,10));
  const exit=new Promise(r=>child.on('exit',r));child.stdin.end(JSON.stringify({id:randomUUID(),op:'open'})+'\n');assert.equal(await exit,0,err);
  const replies=out.trim().split('\n').map(JSON.parse);assert.equal(replies[0].ok,true);assert.equal(replies[1].error.code,'parent_disconnected');await assert.rejects(fs.stat(path.join(root,'worker.lock')));
 }finally{child.kill();await fs.rm(root,{recursive:true,force:true});}
});
