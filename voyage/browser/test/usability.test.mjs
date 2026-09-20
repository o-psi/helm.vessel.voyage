import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import {createHash,randomUUID} from 'node:crypto';
import {spawn} from 'node:child_process';
import {Worker} from '../worker.mjs';
const executable=process.env.CHROMIUM||'/usr/bin/chromium';
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const callFor=w=>async(op,args={})=>{const r=await w.request({id:randomUUID(),op,...w.status(),...args});assert.equal(r.ok,true,JSON.stringify(r));return r.result.value;};

test('two isolated workers: upload/download, key, viewport, tabs, proxied WebSocket',{timeout:90000},async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'worker-usability-'));const a=new Worker(),b=new Worker();const sockets=new Set();let upgrades=0;
 const server=http.createServer((req,res)=>{if(req.url==='/download'){res.writeHead(200,{'Content-Disposition':'attachment; filename="synthetic.txt"'});res.end('download sentinel');return;}res.setHeader('Content-Type','text/html');res.end('<body><input type="file" aria-label="upload"><input aria-label="text"><a href="/download">Download</a></body>');});
 server.on('connection',s=>{sockets.add(s);s.on('close',()=>sockets.delete(s));});
 server.on('upgrade',(req,s)=>{upgrades++;assert.equal(req.headers['proxy-authorization'],undefined);const accept=createHash('sha1').update(req.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');s.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);s.write(Buffer.concat([Buffer.from([0x81,8]),Buffer.from('ws works')]));});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));const origin=`http://127.0.0.1:${server.address().port}`;
 t.after(async()=>{await Promise.all([a.dispose(),b.dispose()]);for(const s of sockets)s.destroy();await new Promise(r=>server.close(r));await fs.rm(root,{recursive:true,force:true});});
 const ca=callFor(a),cb=callFor(b);
 for(const [w,call,name]of [[a,ca,'a'],[b,cb,'b']]){await call('init',{config:{root:path.join(root,name),executable,public_web:false,origins:[{origin,private_network:true},{origin:origin.replace('http:','https:'),private_network:true}],ice_servers:[],width:640,height:480}});await call('open');await call('agent',{action:{kind:'navigate',url:origin}});}
 assert.notEqual(a.browser,b.browser);assert.notEqual(a.task,b.task);
 await a.page.evaluate(()=>{localStorage.setItem('isolation','A');document.cookie='isolation=A';});
 assert.equal(await b.page.evaluate(()=>localStorage.getItem('isolation')),null);assert.equal(await b.page.evaluate(()=>document.cookie),'');
 const mismatch=await b.request({id:randomUUID(),op:'agent',browser:a.browser,epochs:a.epochs,action:{kind:'inspect'}});assert.equal(mismatch.error.code,'stale_binding');
 let inspection=await ca('agent',{action:{kind:'inspect'}});const upload=inspection.elements.find(e=>e.text==='upload').ref;
 await ca('agent',{action:{kind:'upload',ref:upload,name:'synthetic.txt',mime_type:'text/plain',data_base64:Buffer.from('upload sentinel').toString('base64')}});
 assert.deepEqual(await a.page.locator('input[type=file]').evaluate(async e=>[e.files[0].name,await e.files[0].text()]),['synthetic.txt','upload sentinel']);
 assert.equal(await b.page.locator('input[type=file]').evaluate(e=>e.files.length),0);
 const bad=await a.request({id:randomUUID(),op:'agent',...a.status(),action:{kind:'upload',ref:upload,name:'../escape',mime_type:'text/plain',data_base64:''}});assert.equal(bad.error.code,'invalid_filename');
 await ca('agent',{action:{kind:'click',ref:inspection.elements.find(e=>e.tag==='a').ref}});
 for(let i=0;i<50&&!a.downloads.size;i++)await sleep(50);assert.equal(a.downloads.size,1);assert.equal(b.downloads.size,0);
 const download=await ca('agent',{action:{kind:'download',download_id:[...a.downloads.keys()][0]}});assert.equal(Buffer.from(download.data_base64,'base64').toString(),'download sentinel');
 const viewer=randomUUID();await ca('join',{viewer});await ca('control',{viewer,mode:'human'});await a.page.locator('input[aria-label=text]').focus();
 for(const [seq,action]of [[1,{kind:'key',type:'down',key:'a'}],[2,{kind:'key',type:'up',key:'a'}],[3,{kind:'resize',width:800,height:600}]])await ca('input',{viewer,seq,action});
 assert.equal(await a.page.locator('input[aria-label=text]').inputValue(),'a');assert.deepEqual(a.page.viewportSize(),{width:800,height:600});assert.deepEqual(b.page.viewportSize(),{width:640,height:480});
 await ca('control',{viewer,mode:'agent'});
 assert.equal((await a.request({id:randomUUID(),op:'agent',...a.status(),action:{kind:'tabs',operation:'close',tab:a.active}})).error.code,'last_tab');
 await ca('agent',{action:{kind:'tabs',operation:'new'}});assert.deepEqual(a.page.viewportSize(),{width:800,height:600});await ca('agent',{action:{kind:'tabs',operation:'close',tab:a.active}});assert.equal(a.tabs.size,1);assert.equal(a.page.isClosed(),false);
 const wsURL=origin.replace('http:','ws:');
 assert.equal(await a.page.evaluate(url=>new Promise(resolve=>{const ws=globalThis.testSocket=new WebSocket(url);const timer=setTimeout(()=>resolve('timeout'),4000);ws.onmessage=e=>{clearTimeout(timer);resolve(e.data);};ws.onerror=()=>{clearTimeout(timer);resolve('error');};}),wsURL),'ws works');assert.equal(upgrades,1);
 await ca('policy',{public_web:true,origins:[]});
 await a.page.waitForFunction(()=>testSocket.readyState===WebSocket.CLOSED);
 assert.equal(await a.page.evaluate(url=>new Promise(resolve=>{const ws=new WebSocket(url);const timer=setTimeout(()=>resolve('timeout'),4000);ws.onopen=()=>{clearTimeout(timer);resolve('open');};ws.onerror=()=>{clearTimeout(timer);resolve('denied');};}),wsURL),'denied');assert.equal(upgrades,1);
 await ca('close');assert.match((await cb('agent',{action:{kind:'inspect'}})).text,/Download/);
});

test('stdio admits bounded concurrent requests and shuts down cleanly',{timeout:15000},async()=>{
 const child=spawn(process.execPath,[new URL('../worker.mjs',import.meta.url).pathname],{stdio:['pipe','pipe','pipe']});let output='',stderr='';child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>stderr+=b);
 const exited=new Promise((resolve,reject)=>{child.on('error',reject);child.on('exit',(code,signal)=>resolve({code,signal}));});
 const ids=Array.from({length:32},()=>randomUUID());child.stdin.end(ids.map(id=>JSON.stringify({id,op:'status'})).join('\n')+'\n');
 assert.deepEqual(await exited,{code:0,signal:null});const replies=output.trim().split('\n').map(JSON.parse);assert.equal(replies.length,32);assert.deepEqual(new Set(replies.map(r=>r.id)),new Set(ids));assert.ok(replies.every(r=>r.error.code==='not_initialized'));assert.equal(stderr,'');
});

test('human browser chrome exposes titles/history and preserves navigation fences',{timeout:30000},async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'browser-chrome-'));const w=new Worker();
 const server=http.createServer((req,res)=>{res.setHeader('Content-Type','text/html');res.end(`<title>${req.url==='/one'?'First page':'Second page'}</title><h1>Browser chrome</h1>`);});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));const origin=`http://127.0.0.1:${server.address().port}`;
 t.after(async()=>{await w.dispose();await new Promise(r=>server.close(r));await fs.rm(root,{recursive:true,force:true});});
 const call=callFor(w);await call('init',{config:{root,executable,public_web:false,origins:[{origin,private_network:true}],ice_servers:[]}});await call('open');
 await call('agent',{action:{kind:'navigate',url:origin+'/one'}});await call('agent',{action:{kind:'navigate',url:origin+'/two'}});
 assert.equal(w.status().page.title,'Second page');assert.ok(w.status().page.can_go_back);assert.equal(w.status().tab_details[0].id,w.active);
 const viewer=randomUUID();await call('join',{viewer});await call('control',{viewer,mode:'private'});
 await call('input',{viewer,seq:1,action:{kind:'history',direction:'back'}});assert.equal(w.status().page.url,origin+'/one');assert.equal(w.status().page.title,'First page');assert.ok(w.status().page.can_go_forward);
 await call('input',{viewer,seq:2,action:{kind:'history',direction:'forward'}});assert.equal(w.status().page.url,origin+'/two');
 await call('input',{viewer,seq:3,action:{kind:'history',direction:'reload'}});assert.equal(w.status().page.title,'Second page');
});
