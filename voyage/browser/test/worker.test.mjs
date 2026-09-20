import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import {randomUUID} from 'node:crypto';
import {chromium} from 'playwright-core';
import {Worker} from '../worker.mjs';
import {publicAddress} from '../security.mjs';
const executable=process.env.CHROMIUM||'/usr/bin/chromium';
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
test('address classification',()=>{for(const ip of ['127.0.0.1','::1','::ffff:127.0.0.1','10.1.2.3','169.254.169.254','100.64.0.1','192.168.1.1','192.0.0.1','192.0.2.1','192.88.99.1','198.18.0.1','198.19.255.254','198.51.100.1','203.0.113.1','2001::1','2001:01ff::1','2001:0db8::1','2002:0808:0808::1','3fff::1'])assert.equal(publicAddress(ip),false);for(const ip of ['8.8.8.8','192.0.43.8','192.0.3.1','192.0.255.254','2001:500:88:200::8','2001:0200::1','2606:4700:4700::1111'])assert.equal(publicAddress(ip),true);});
test('sandboxed worker, decoded media, interruption, fences and receipts',{timeout:90000},async t=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'worker333-'));const w=new Worker();let receiver;
 const sockets=new Set();const server=http.createServer((req,res)=>{if(req.url==='/blocked')return;res.setHeader('Content-Type','text/html');res.end(`<body style="background:#159;color:white"><h1>Synthetic worker</h1><input aria-label="entry"><button onclick="document.querySelector('h1').textContent='clicked'">Click</button><button onclick="alert('synthetic dialog')">Dialog</button><canvas width="320" height="160"></canvas></body>`);});
 server.on('connection',s=>{sockets.add(s);s.on('close',()=>sockets.delete(s));});await new Promise(r=>server.listen(0,'127.0.0.1',r));const origin=`http://127.0.0.1:${server.address().port}`;
 t.after(async()=>{await receiver?.close();await w.dispose();for(const s of sockets)s.destroy();await new Promise(r=>server.close(r));await fs.rm(root,{recursive:true,force:true});});
 const call=async(op,args={})=>{const result=await w.request({id:randomUUID(),op,...w.status(),...args});assert.equal(result.ok,true,JSON.stringify(result));return result.result;};
 // Do not spread status.op or any runtime authority into page arguments.
 await call('init',{config:{root,executable,public_web:true,origins:[{origin,private_network:true}],ice_servers:[],width:640,height:480}});
 await call('open');await call('agent',{action:{kind:'navigate',url:origin}});
 let inspection=(await call('agent',{action:{kind:'inspect'}})).value;assert.match(inspection.text,/Synthetic worker/);assert.equal(inspection.elements.length,3);
 await call('agent',{action:{kind:'fill',ref:inspection.elements[0].ref,text:'synthetic'}});assert.equal(await w.page.locator('input').inputValue(),'synthetic');
 const sandbox=await w.task.newPage();await sandbox.goto('chrome://sandbox');const sandboxText=await sandbox.locator('body').innerText();assert.match(sandboxText,/Seccomp-BPF sandbox\s+Yes/);await sandbox.close();
 const viewer=randomUUID(),other=randomUUID();await call('join',{viewer});await call('join',{viewer:other});
 receiver=await chromium.launch({executablePath:executable,headless:true,chromiumSandbox:true,args:['--disable-features=WebRtcHideLocalIpsWithMdns','--disable-background-networking','--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1']});const rp=await receiver.newPage();
 const priorDocument={...w.epochs};await call('agent',{action:{kind:'navigate',url:origin+'/next'}});
 const offer=(await call('offer',{viewer,signal_seq:1,epochs:priorDocument})).value;
 const answer=await rp.evaluate(async offer=>{const pc=globalThis.pc=new RTCPeerConnection({iceServers:[]});const video=document.createElement('video');video.autoplay=true;video.muted=true;document.body.append(video);pc.ontrack=e=>{video.srcObject=e.streams[0];void video.play();};await pc.setRemoteDescription(offer);await pc.setLocalDescription(await pc.createAnswer());await new Promise(r=>{if(pc.iceGatheringState==='complete')return r();pc.onicegatheringstatechange=()=>{if(pc.iceGatheringState==='complete')r();};});return {type:pc.localDescription.type,sdp:pc.localDescription.sdp};},offer);
 // Delay answer beyond initial static screencast frames: every new viewer still needs pixels.
 await sleep(700);await call('answer',{viewer,signal_seq:2,description:answer});
 let decoded=0;for(let i=0;i<60;i++){decoded=await rp.evaluate(async()=>{let n=0;for(const s of (await pc.getStats()).values())if(s.type==='inbound-rtp')n+=s.framesDecoded||0;return n;});if(decoded>5)break;await sleep(100);}assert.ok(decoded>5,`decoded ${decoded}`);console.log('decoded frames',decoded);
 const rp2=await receiver.newPage();
 const offer2=(await call('offer',{viewer:other,signal_seq:1})).value;
 const answer2=await rp2.evaluate(async offer=>{const pc=globalThis.pc=new RTCPeerConnection({iceServers:[]});const video=document.createElement('video');video.autoplay=true;video.muted=true;document.body.append(video);pc.ontrack=e=>{video.srcObject=e.streams[0];void video.play();};await pc.setRemoteDescription(offer);await pc.setLocalDescription(await pc.createAnswer());await new Promise(r=>{if(pc.iceGatheringState==='complete')return r();pc.onicegatheringstatechange=()=>{if(pc.iceGatheringState==='complete')r();};});return {type:pc.localDescription.type,sdp:pc.localDescription.sdp};},offer2);
 await call('answer',{viewer:other,signal_seq:2,description:answer2});
 let decoded2=0;for(let i=0;i<60;i++){decoded2=await rp2.evaluate(async()=>{let n=0;for(const s of(await pc.getStats()).values())if(s.type==='inbound-rtp')n+=s.framesDecoded||0;return n;});if(decoded2>5)break;await sleep(100);}assert.ok(decoded2>5,`second viewer decoded ${decoded2}`);console.log('second viewer decoded frames',decoded2);
 assert.equal(await w.encoder.evaluate(()=>encoder.peerCount()),2);
 const blocked=w.request({id:randomUUID(),op:'agent',browser:w.browser,epochs:{...w.epochs},action:{kind:'navigate',url:origin+'/blocked'}});await sleep(200);
 const started=Date.now();await call('control',{viewer,mode:'private'});assert.ok(Date.now()-started<3000);assert.equal((await blocked).error.code,'control_fenced');assert.deepEqual(w.status().viewers,[viewer]);
 assert.equal((await w.request({id:randomUUID(),op:'agent',browser:w.browser,epochs:{...w.epochs},action:{kind:'inspect'}})).error.code,'agent_fenced');
 assert.equal(await w.encoder.evaluate(()=>encoder.peerCount()),0);
 // Hold an actual JPEG decode across reset; late bitmap completion must not draw.
 const jpeg=(await w.page.screenshot({type:'jpeg'})).toString('base64');
 const generation=w.epochs.capture;
 await w.encoder.evaluate(()=>{const original=globalThis.createImageBitmap;globalThis.createImageBitmap=async(...args)=>{const bitmap=await original(...args);await new Promise(r=>globalThis.releaseBitmap=r);return bitmap;};});
 const stale=w.encoder.evaluate(f=>encoder.frame(f),{data:jpeg,width:640,height:480,generation});
 await w.encoder.waitForFunction(()=>typeof releaseBitmap==='function');
 await w.encoder.evaluate(g=>encoder.reset(g),generation+1);
 await w.encoder.evaluate(()=>releaseBitmap());await stale;
 assert.equal(await w.encoder.evaluate(()=>document.querySelector('canvas').getContext('2d').getImageData(10,10,1,1).data[3]),0);
 await w.encoder.evaluate(g=>encoder.reset(g),generation);

 await call('input',{viewer,seq:1,action:{kind:'navigate',url:origin}});
 // A native modal blocks click completion; the dialog interrupt lane must still run.
 const box=await w.page.locator('button').nth(1).boundingBox();
 await call('input',{viewer,seq:2,action:{kind:'pointer',type:'down',x:Math.round(box.x+box.width/2),y:Math.round(box.y+box.height/2),button:'left'}});
 const pending=w.request({id:randomUUID(),op:'input',browser:w.browser,epochs:{...w.epochs},viewer,seq:3,action:{kind:'pointer',type:'up',x:Math.round(box.x+box.width/2),y:Math.round(box.y+box.height/2),button:'left'}});for(let i=0;i<30&&!w.dialog;i++)await sleep(50);assert.ok(w.dialog);
 await call('input',{viewer,seq:4,action:{kind:'dialog',accept:true}});assert.equal((await pending).ok,true);
 const request={id:randomUUID(),op:'input',browser:w.browser,epochs:{...w.epochs},viewer,seq:5,action:{kind:'text',text:'PRIVATE-SENTINEL'}};
 assert.equal((await w.request(request)).ok,true);assert.equal((await w.request(request)).result.content_withheld,true);assert.equal((await w.request({...request,seq:6})).error.code,'id_conflict');
 await call('disconnect',{viewer});assert.equal(w.mode,'private');assert.equal(w.controller,viewer);await call('join',{viewer});await call('control',{viewer,mode:'agent'});
 const receipts=await fs.readdir(path.join(root,'receipts'));for(const file of receipts)assert.doesNotMatch(await fs.readFile(path.join(root,'receipts',file),'utf8'),/PRIVATE-SENTINEL/);
 await call('agent',{action:{kind:'tabs',operation:'new'}});assert.equal(w.tabs.size,2);
 await call('close');assert.equal(w.task,null);assert.equal(w.encoding,null);
});
