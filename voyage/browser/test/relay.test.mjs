import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import net from 'node:net';
import {randomBytes,randomUUID} from 'node:crypto';
import {spawn} from 'node:child_process';
import {chromium} from 'playwright-core';
import {Worker} from '../worker.mjs';
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
// Opt-in local qualification. A missing configured executable fails, never passes.
// TURN_TEST_ROOT must be an ignored evidence directory; no installation is needed.
test('actual TURN relay-only selected pairs carry decoded worker video',{
 timeout:60000,skip:process.env.TURN_SERVER?false:'TURN_SERVER unset: relay qualification NOT performed',
},async t=>{
 assert.ok(process.env.TURN_TEST_ROOT,'TURN_TEST_ROOT is required');
 const binary=path.resolve(process.env.TURN_SERVER);await fs.access(binary,fs.constants.X_OK);
 const root=await fs.mkdtemp(path.join(path.resolve(process.env.TURN_TEST_ROOT),'run-'));
 const w=new Worker();let receiver,turn,exit;
 t.after(async()=>{
  try{await receiver?.close();}finally{try{await w.dispose();}finally{
   if(turn&&turn.exitCode===null&&turn.signalCode===null){turn.kill('SIGTERM');await Promise.race([exit,sleep(2000)]);if(turn.exitCode===null&&turn.signalCode===null)turn.kill('SIGKILL');await exit;}
   await fs.rm(root,{recursive:true,force:true});
  }}
 });
 // Reserve an ephemeral loopback TCP port, then release it for coturn TCP/UDP.
 const probe=net.createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));const port=probe.address().port;await new Promise(r=>probe.close(r));
 const username=randomBytes(16).toString('hex'),credential=randomBytes(32).toString('hex');
 const config=path.join(root,'turn.conf');
 await fs.writeFile(config,[`listening-port=${port}`,'listening-ip=127.0.0.1','relay-ip=127.0.0.1','realm=browser333.invalid','lt-cred-mech',`user=${username}:${credential}`,'no-cli','no-tls','no-dtls','no-multicast-peers','allow-loopback-peers','denied-peer-ip=0.0.0.0-255.255.255.255','allowed-peer-ip=127.0.0.1','denied-peer-ip=::-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff','no-tcp-relay','relay-threads=1','no-software-attribute','no-stdout-log','log-file=/dev/null',`pidfile=${path.join(root,'turn.pid')}`,`userdb=${path.join(root,'unused.sqlite')}`].join('\n')+'\n',{mode:0o600});
 // Credentials are only in this short-lived mode-0600 file, not argv or logs.
 turn=spawn(binary,['-c',config],{stdio:'ignore'});exit=new Promise((resolve,reject)=>{turn.once('error',reject);turn.once('exit',(code,signal)=>resolve({code,signal}));});
 let ready=false;for(let i=0;i<50;i++){
  assert.equal(turn.exitCode,null,'coturn exited before readiness');
  ready=await new Promise(r=>{const s=net.connect(port,'127.0.0.1');s.once('connect',()=>{s.destroy();r(true);});s.once('error',()=>r(false));});if(ready)break;await sleep(100);
 }assert.ok(ready,'loopback TURN listener not ready');
 const executable=process.env.CHROMIUM||'/usr/bin/chromium';
 const iceServers=[{urls:[`turn:127.0.0.1:${port}?transport=udp`],username,credential}];
 const call=async(op,args={})=>{const r=await w.request({id:randomUUID(),op,...w.status(),...args});assert.equal(r.ok,true,`worker ${op}: ${r.error?.code}`);return r.result.value;};
 await call('init',{config:{root:path.join(root,'worker'),executable,public_web:false,origins:[],ice_servers:iceServers,relay_only:true,width:640,height:480}});
 await call('open');
 await w.page.setContent('<body style="background:#159"><h1>Local relay qualification</h1><canvas width="320" height="160"></canvas><script>let i=0;setInterval(()=>{const c=document.querySelector("canvas").getContext("2d");c.fillStyle=i++%2?"red":"blue";c.fillRect(0,0,320,160)},50)</script>');
 // Observe the real encoder peer without altering configuration or network flags.
 await w.encoder.evaluate(()=>{const Original=RTCPeerConnection;globalThis.RTCPeerConnection=class extends Original{constructor(config){super(config);globalThis.observedPeer=this;}};});
 const viewer=randomUUID();await call('join',{viewer});
 receiver=await chromium.launch({executablePath:executable,headless:true,chromiumSandbox:true,args:['--disable-features=WebRtcHideLocalIpsWithMdns','--disable-background-networking','--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1']});
 const rp=await receiver.newPage();const offer=await call('offer',{viewer,signal_seq:1});
 assert.match(offer.sdp,/ typ relay/,'encoder must gather a relay candidate');
 assert.doesNotMatch(offer.sdp,/ typ (host|srflx) /,'encoder must not signal direct candidates');
 const answer=await rp.evaluate(async({offer,iceServers})=>{
  const pc=globalThis.observedPeer=new RTCPeerConnection({iceServers,iceTransportPolicy:'relay'});
  const video=document.createElement('video');video.autoplay=true;video.muted=true;document.body.append(video);pc.ontrack=e=>{video.srcObject=e.streams[0];void video.play();};
  await pc.setRemoteDescription(offer);await pc.setLocalDescription(await pc.createAnswer());
  await new Promise((resolve,reject)=>{if(pc.iceGatheringState==='complete')return resolve();const timer=setTimeout(()=>reject(Error('receiver ICE timeout')),8000);pc.onicegatheringstatechange=()=>{if(pc.iceGatheringState==='complete'){clearTimeout(timer);resolve();}};});
  return {type:pc.localDescription.type,sdp:pc.localDescription.sdp};
 },{offer,iceServers});
 await call('answer',{viewer,signal_seq:2,description:answer});
 const stats=async page=>page.evaluate(async()=>{
  const pc=observedPeer,report=await pc.getStats();let frames=0,bytes=0,pair;
  for(const s of report.values()){if(s.type==='inbound-rtp'){frames+=s.framesDecoded||0;bytes+=s.bytesReceived||0;}if(s.type==='transport'&&s.selectedCandidatePairId)pair=report.get(s.selectedCandidatePairId);}
  return {policy:pc.getConfiguration().iceTransportPolicy,state:pc.connectionState,frames,bytes,pair:pair&&{state:pair.state,nominated:pair.nominated,bytesSent:pair.bytesSent,bytesReceived:pair.bytesReceived,local:report.get(pair.localCandidateId)?.candidateType,remote:report.get(pair.remoteCandidateId)?.candidateType}};
 });
 let received;for(let i=0;i<120;i++){received=await stats(rp);if(received.frames>5)break;await sleep(100);}
 assert.ok(received.frames>5,`decoded frames: ${received.frames}`);
 const before=received.frames;await sleep(500);received=await stats(rp);assert.ok(received.frames>before,'decoded video must continue advancing');
 const sent=await stats(w.encoder);
 for(const s of [sent,received]){assert.equal(s.policy,'relay');assert.equal(s.state,'connected');assert.equal(s.pair?.state,'succeeded');assert.equal(s.pair?.local,'relay');assert.equal(s.pair?.remote,'relay');}
 assert.ok(received.bytes>0);assert.ok(sent.pair.bytesSent>0);
 console.log('TURN qualification (sanitized)',JSON.stringify({encoder:sent,receiver:received}));
});
