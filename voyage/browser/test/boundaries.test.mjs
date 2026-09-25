import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import {spawn} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import {networkProxy,digest} from '../security.mjs';
import {Journal} from '../journal.mjs';
import {Worker} from '../worker.mjs';
test('public-web proxy denies loopback unless explicitly granted; replacement revokes',async()=>{
 const server=http.createServer((_,r)=>r.end('fixture'));await new Promise(r=>server.listen(0,'127.0.0.1',r));const url=`http://127.0.0.1:${server.address().port}`;
 const proxy=await networkProxy(new Map(),true);
 const get=()=>new Promise((resolve,reject)=>{const u=new URL(proxy.server);http.get({agent:false,host:u.hostname,port:u.port,path:url,headers:{'Proxy-Authorization':`Basic ${Buffer.from(`${proxy.username}:${proxy.password}`).toString('base64')}`}},res=>{res.resume();res.on('end',()=>resolve(res.statusCode));}).on('error',reject);});
 try{assert.equal(await get(),403);proxy.update(new Map([[url,{private_network:true}]]),true);assert.equal(await get(),200);proxy.update(new Map(),true);assert.equal(await get(),403);}finally{await proxy.close();await new Promise(r=>server.close(r));}
});
test('durable crash receipt never replays, changed IDs refuse',async()=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'journal333-'));try{const j=new Journal(root);await j.init();const req={id:randomUUID(),op:'agent',text:'secret'};await j.begin(req);const loaded=new Journal(root);await loaded.init();assert.equal(loaded.previous(req).state,'unknown');assert.throws(()=>loaded.previous({...req,text:'changed'}),/id_conflict/);assert.doesNotMatch(await fs.readFile(path.join(root,req.id+'.json'),'utf8'),/secret/);}finally{await fs.rm(root,{recursive:true,force:true});}
});
test('unobserved cleanup retains ownership lock',async()=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'cleanup333-'));const w=new Worker();try{const r=await w.request({id:randomUUID(),op:'init',config:{root,executable:'/usr/bin/chromium',public_web:false,origins:[]}});assert.equal(r.ok,true);w.task={close:async()=>{throw Error('unobserved');}};await assert.rejects(w.dispose(),/cleanup_failed/);await fs.stat(path.join(root,'worker.lock'));w.task=null;await w.dispose();}finally{await fs.rm(root,{recursive:true,force:true});}
});
test('stdio JSON only, malformed input bounded, clean shutdown',{timeout:15000},async()=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'stdio333-'));const child=spawn(process.execPath,['worker.mjs'],{cwd:new URL('..',import.meta.url),stdio:['pipe','pipe','pipe']});let out='',err='';child.stdout.on('data',c=>out+=c);child.stderr.on('data',c=>err+=c);
 child.stdin.write('not json\n');child.stdin.write(JSON.stringify({id:randomUUID(),op:'init',config:{root,executable:'/usr/bin/chromium',public_web:false,origins:[]}})+'\n');
 await new Promise(r=>setTimeout(r,500));child.stdin.write(JSON.stringify({id:randomUUID(),op:'shutdown'})+'\n');child.stdin.end();
 const code=await new Promise(r=>child.on('exit',r));assert.equal(code,0,err);const replies=out.trim().split('\n').map(JSON.parse);assert.equal(replies.length,3);assert.equal(replies[0].error.code,'invalid_json');assert.equal(replies[1].ok,true);assert.equal(replies[2].ok,false);assert.equal(replies[2].error.code,'parent_disconnected');await assert.rejects(fs.stat(path.join(root,'worker.lock')));await fs.rm(root,{recursive:true,force:true});
});
test('input ledger stays bounded without durable per-input writes; evicted sequences refuse',async()=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'input333-'));const w=new Worker();
 try{
  assert.equal((await w.request({id:randomUUID(),op:'init',config:{root,executable:'/usr/bin/chromium',public_web:false,origins:[]}})).ok,true);
  const viewer=randomUUID();w.viewers.set(viewer,{seq:0});w.controller=viewer;w.mode='human';w.task={close:async()=>{}};w.page={isClosed:()=>false,keyboard:{insertText:async()=>{}}};
  let first;
  for(let seq=1;seq<=2100;seq++){const req={id:randomUUID(),op:'input',browser:w.browser,epochs:{...w.epochs},viewer,seq,action:{kind:'text',text:'synthetic'}};if(seq===1)first=req;assert.equal((await w.request(req)).ok,true);}
  assert.equal(w.ephemeral.size,2048);assert.equal((await fs.readdir(path.join(root,'receipts'))).length,1);
  assert.equal((await w.request(first)).error.code,'input_sequence');
 }finally{w.page=null;await w.dispose();await fs.rm(root,{recursive:true,force:true});}
});
