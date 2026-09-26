import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import {gunzipSync} from 'node:zlib';
import {randomUUID} from 'node:crypto';
import {Worker} from '../worker.mjs';
import {publicAddress} from '../security.mjs';
const executable=process.env.CHROMIUM||'/usr/bin/chromium';
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));

test('address classification',()=>{
 for(const ip of ['127.0.0.1','::1','::ffff:127.0.0.1','10.1.2.3','169.254.169.254','100.64.0.1','192.168.1.1','192.0.0.1','192.0.2.1','192.88.99.1','198.18.0.1','198.19.255.254','198.51.100.1','203.0.113.1','2001::1','2001:01ff::1','2001:0db8::1','2002:0808:0808::1','3fff::1'])assert.equal(publicAddress(ip),false);
 for(const ip of ['8.8.8.8','192.0.43.8','192.0.3.1','192.0.255.254','2001:500:88:200::8','2001:0200::1','2606:4700:4700::1111'])assert.equal(publicAddress(ip),true);
});

test('replay resources use host-captured bytes and never retain stylesheet URLs',()=>{
 const worker=new Worker();
 worker.assets.set('https://site.example/static/logo.png','data:image/png;base64,AA==');
 const events=[{type:2,data:{node:{type:0,childNodes:[{type:2,tagName:'link',attributes:{href:'https://site.example/static/site.css',_cssText:'body{background:url(logo.png)}'},childNodes:[]},{type:2,tagName:'img',attributes:{src:'https://unknown.example/track.png'},childNodes:[]},{type:2,tagName:'meta',attributes:{'http-equiv':'refresh',content:'0;url=https://unknown.example/'},childNodes:[]},{type:2,tagName:'iframe',attributes:{srcdoc:'<script>location="https://unknown.example"</script>'},childNodes:[]}]}}}];
 worker.inlineAssets(events,'https://site.example/page');
 const [stylesheet,image]=events[0].data.node.childNodes;
 assert.equal(stylesheet.attributes.href,undefined);
 assert.match(stylesheet.attributes._cssText,/data:image\/png;base64,AA==/);
 assert.equal(image.attributes.src,'');
 assert.equal(events[0].data.node.childNodes[2].attributes.content,undefined);
 assert.equal(events[0].data.node.childNodes[3].attributes.srcdoc,undefined);
});

test('tab selection stops old frame recorders and clears viewer cursors',async()=>{
 const worker=new Worker(),stopped=[];
 const page=name=>({isClosed:()=>false,frames:()=>[{evaluate:async()=>{stopped.push(name);}}],
  setViewportSize:async()=>{},bringToFront:async()=>{}});
 const old=page('old'),next=page('next'),oldId=randomUUID(),nextId=randomUUID();
 worker.config={width:640,height:480};worker.tabs.set(oldId,old);worker.tabs.set(nextId,next);
 worker.page=old;worker.active=oldId;
 const viewer={seq:0,mirrorCursor:19,frameCursors:new Map([['child',7]])};worker.viewers.set('viewer',viewer);
 await worker.select(nextId);
 assert.deepEqual(stopped,['old','next']);
 assert.equal(worker.page,next);
 assert.equal(viewer.mirrorCursor,0);
 assert.equal(viewer.frameCursors.size,0);
});

test('one sandboxed browser serves bounded live DOM to authorized viewers',{timeout:90000},async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'worker333-'));
 const worker=new Worker(),sockets=new Set();
 const server=http.createServer((req,res)=>{
  if(req.url==='/blocked')return;
  res.setHeader('Content-Type','text/html');
  res.end(`<body style="background:#159;color:white"><h1>Synthetic worker</h1><input aria-label="entry"><button onclick="document.querySelector('h1').textContent='clicked'">Click</button><button onclick="alert('synthetic dialog')">Dialog</button><canvas width="320" height="160" onclick="document.querySelector('h1').textContent='surface'"></canvas></body>`);
 });
 server.on('connection',socket=>{sockets.add(socket);socket.on('close',()=>sockets.delete(socket));});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 const origin=`http://127.0.0.1:${server.address().port}`;
 t.after(async()=>{await worker.dispose();for(const socket of sockets)socket.destroy();await new Promise(resolve=>server.close(resolve));await fs.rm(root,{recursive:true,force:true});});
 const request=(op,args={})=>worker.request({id:randomUUID(),op,browser:worker.browser,epochs:{...worker.epochs},...args});
 const call=async(op,args={})=>{const response=await request(op,args);assert.equal(response.ok,true,JSON.stringify(response));return response.result;};
 const events=result=>JSON.parse(gunzipSync(Buffer.from(result.value.data_base64,'base64')).toString());
 await call('init',{config:{root,executable,public_web:true,origins:[{origin,private_network:true}],width:640,height:480}});
 await call('open');await call('agent',{action:{kind:'navigate',url:origin}});
 const sandbox=await worker.task.newPage();await sandbox.goto('chrome://sandbox');
 assert.match(await sandbox.locator('body').innerText(),/Seccomp-BPF sandbox\s+Yes/);await sandbox.close();
 const viewer=randomUUID(),other=randomUUID();await call('join',{viewer});await call('join',{viewer:other});
 const first=await call('mirror',{viewer,since:0});
 assert.equal(first.value.reset,true);assert.match(JSON.stringify(events(first)),/Synthetic worker/);
 assert.ok(events(first).every(event=>event&&Number.isInteger(event.type)));
 assert.ok(events(first).some(event=>event.type===4));
 assert.ok(events(first).some(event=>event.type===2));
 assert.equal(first.value.visuals?.length,1,'visible canvas receives a localized image');
 assert.ok(first.value.visuals[0].id>0&&first.value.visuals[0].data_base64.length>0);
 await worker.page.evaluate(()=>{document.body.style.height='1400px';const canvas=document.querySelector('canvas');canvas.style.position='absolute';canvas.style.top='850px';scrollTo(0,780);});
 worker.visualAt=0;
 const scrolled=await call('mirror',{viewer,since:first.value.cursor});
 assert.equal(scrolled.value.visuals.length,1,'a canvas still captures after page scrolling');
 await worker.page.evaluate(()=>{scrollTo(0,0);const canvas=document.querySelector('canvas');canvas.style.position='';canvas.style.top='';document.body.style.height='';});
 worker.visualAt=0;
 assert.equal(worker.task.contexts().length,1,'the task browser has one context and no encoder browser');
 const second=await call('mirror',{viewer:other,since:0});assert.match(JSON.stringify(events(second)),/Synthetic worker/);
 const blocked=request('agent',{action:{kind:'navigate',url:origin+'/blocked'}});await sleep(200);
 const started=Date.now();await call('control',{viewer,mode:'private'});assert.ok(Date.now()-started<3000);
 assert.equal((await blocked).error.code,'control_fenced');assert.deepEqual(worker.status().viewers,[viewer]);
 assert.equal((await request('mirror',{viewer:other,since:0})).error.code,'viewer_missing');
 assert.equal((await request('agent',{action:{kind:'inspect'}})).error.code,'agent_fenced');
 const privateView=await call('mirror',{viewer,since:0});assert.equal(privateView.value.reset,true);
 const buttonId=await worker.page.evaluate(()=>globalThis.__voyageMirror.id(document.querySelector('button')));
 await call('input',{viewer,seq:1,action:{kind:'element',action:'click',node_id:buttonId,button:'left'}});
 assert.equal(await worker.page.locator('h1').innerText(),'clicked');
 const changed=await call('mirror',{viewer,since:privateView.value.cursor});assert.match(JSON.stringify(events(changed)),/clicked/);
 const canvasId=await worker.page.evaluate(()=>globalThis.__voyageMirror.id(document.querySelector('canvas')));
 await call('input',{viewer,seq:2,action:{kind:'element',action:'surface_click',node_id:canvasId,x:5000,y:5000,button:'left'}});
 assert.equal(await worker.page.locator('h1').innerText(),'surface');
 await call('input',{viewer,seq:3,action:{kind:'resize',width:400,height:600}});
 assert.deepEqual(worker.status().viewport,{width:400,height:600});
 const requestId=randomUUID();const input={id:requestId,op:'input',browser:worker.browser,epochs:{...worker.epochs},viewer,seq:4,action:{kind:'text',text:'PRIVATE-SENTINEL'}};
 assert.equal((await worker.request(input)).ok,true);assert.equal((await worker.request(input)).result.content_withheld,true);
 assert.equal((await worker.request({...input,seq:5})).error.code,'id_conflict');
 await call('disconnect',{viewer});assert.equal(worker.mode,'private');assert.equal(worker.controller,viewer);
 await call('join',{viewer});await call('control',{viewer,mode:'agent'});
 const receipts=await fs.readdir(path.join(root,'receipts'));
 for(const file of receipts)assert.doesNotMatch(await fs.readFile(path.join(root,'receipts',file),'utf8'),/PRIVATE-SENTINEL/);
 await call('close');assert.equal(worker.task,null);
});

test('cross-origin frame mirror stays private and frame actions expire on navigation',{timeout:90000},async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'worker333-frame-'));
 const worker=new Worker();let authorizedAssets=0;
 const child=http.createServer((req,res)=>{
  if(req.url==='/private-image.png'){
   if(!req.headers.cookie?.includes('session=asset-ok')){res.writeHead(403).end();return;}
   authorizedAssets++;
   const image=Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/d3sAAAAASUVORK5CYII=','base64');
   res.writeHead(200,{'Content-Type':'image/png','Content-Length':image.length});res.end(image);return;
  }
  res.setHeader('Content-Type','text/html');
  res.end(req.url==='/next'?'<h2>New child document</h2>':req.url==='/nested'?'<h3>Nested cross-origin document</h3><button>Nested action</button><canvas width="80" height="35"></canvas><script>document.querySelector("canvas").getContext("2d").fillRect(0,0,80,35)</script>':'<label>Private code<input aria-label="Private code"></label><button onclick="document.querySelector(\'h2\').textContent=\'Submitted\'">Submit</button><h2>Child form</h2><img src="/private-image.png"><canvas width="80" height="35"></canvas><script>document.querySelector("canvas").getContext("2d").fillRect(0,0,80,35)</script>');
 });
 await new Promise(resolve=>child.listen(0,'127.0.0.1',resolve));
 const childOrigin=`http://127.0.0.1:${child.address().port}`;
 const parent=http.createServer((req,res)=>{
  res.setHeader('Content-Type','text/html');
  if(req.url==='/local'){res.end(`<h2>Local child document</h2><button>Local action</button><iframe src="${childOrigin}/nested" width="200" height="180"></iframe>`);return;}
  res.end(`<h1>Parent</h1><script>window.leaks=[];window.addEventListener('message',e=>leaks.push(JSON.stringify(e.data)))</script><iframe src="${childOrigin}/form" width="400" height="250"></iframe><iframe src="/local" width="250" height="260"></iframe><canvas style="position:fixed;right:0;top:0" width="80" height="35"></canvas>`);
 });
 await new Promise(resolve=>parent.listen(0,'127.0.0.1',resolve));
 const parentOrigin=`http://127.0.0.1:${parent.address().port}`;
 t.after(async()=>{await worker.dispose();await new Promise(resolve=>parent.close(resolve));await new Promise(resolve=>child.close(resolve));await fs.rm(root,{recursive:true,force:true});});
 const request=(op,args={})=>worker.request({id:randomUUID(),op,browser:worker.browser,epochs:{...worker.epochs},...args});
 const call=async(op,args={})=>{const response=await request(op,args);assert.equal(response.ok,true,JSON.stringify(response));return response.result;};
 await call('init',{config:{root,executable,public_web:false,origins:[{origin:parentOrigin,private_network:true},{origin:childOrigin,private_network:true}],width:800,height:600}});
 await call('open');await worker.context.addCookies([{name:'session',value:'asset-ok',url:childOrigin}]);await call('agent',{action:{kind:'navigate',url:parentOrigin}});
 const frame=worker.page.frames()[1];await frame.locator('input').waitFor();
 const viewer=randomUUID(),other=randomUUID();await call('join',{viewer});await call('join',{viewer:other});
 const first=await call('mirror',{viewer,since:0});
 assert.equal(first.value.frames.length,3);
 const childView=first.value.frames[0];
 assert.match(gunzipSync(Buffer.from(childView.data_base64,'base64')).toString(),/Private code/);
 assert.ok(authorizedAssets>0,'the host browser fetched the gated image with its cookie');
 assert.match(gunzipSync(Buffer.from(childView.data_base64,'base64')).toString(),/data:image\/png/);
 assert.equal(childView.visuals.length,1,'canvas inside the child frame gets a localized visual');
 assert.match(gunzipSync(Buffer.from(first.value.frames[1].data_base64,'base64')).toString(),/Local child document/);
 assert.equal(first.value.frames[2].parent_frame_id,first.value.frames[1].frame_id);
 assert.match(gunzipSync(Buffer.from(first.value.frames[2].data_base64,'base64')).toString(),/Nested cross-origin document/);
 assert.equal(first.value.frames[2].visuals.length,1,'nested canvas also gets a localized visual');
 assert.equal(first.value.visuals.some(item=>item.id===childView.host_node_id),false);
 assert.equal(first.value.visuals.length,1,'main canvas is retained after larger mirrored frames are excluded');
 assert.deepEqual(await worker.page.evaluate(()=>window.leaks),[]);
 await call('control',{viewer,mode:'private'});
 assert.equal(worker.frameVisuals.size,0,'control fence clears child media cache');
 assert.equal((await request('mirror',{viewer:other,since:0})).error.code,'viewer_missing');
 const privateView=await call('mirror',{viewer,since:0});
 const dialogSeen=worker.page.waitForEvent('dialog');
 const dialogEffect=frame.evaluate(()=>alert('frame dialog'));
 await dialogSeen;
 const paused=await call('mirror',{viewer,since:privateView.value.cursor});
 assert.deepEqual(paused.value.frames,[]);
 assert.equal(worker.viewers.get(viewer).frameCursors.size,0,'dialog invalidates removed child replay cursors');
 await call('input',{viewer,seq:1,action:{kind:'dialog',accept:false}});await dialogEffect;
 const resumed=await call('mirror',{viewer,since:paused.value.cursor});
 assert.equal(resumed.value.frames.length,3);
 assert.ok(resumed.value.frames.every(item=>item.reset),'child replay restarts with full snapshots after dialog');
 const frameId=resumed.value.frames[0].frame_id;
 const inputId=await frame.evaluate(()=>globalThis.__voyageMirror.id(document.querySelector('input')));
 await call('input',{viewer,seq:2,action:{kind:'element',frame_id:frameId,action:'fill',node_id:inputId,text:'PRIVATE-CODE'}});
 assert.equal(await frame.locator('input').inputValue(),'PRIVATE-CODE');
 assert.deepEqual(await worker.page.evaluate(()=>window.leaks),[]);
 await frame.goto(childOrigin+'/next');
 assert.equal((await request('input',{viewer,seq:3,action:{kind:'element',frame_id:frameId,action:'click',node_id:inputId,button:'left'}})).error.code,'stale_frame');
 const refreshed=await call('mirror',{viewer,since:resumed.value.cursor});
 assert.notEqual(refreshed.value.frames[0].frame_id,frameId);
 assert.match(gunzipSync(Buffer.from(refreshed.value.frames[0].data_base64,'base64')).toString(),/New child document/);
});
