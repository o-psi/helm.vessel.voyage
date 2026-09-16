// Local synthetic evidence only: no providers, production credentials, or browser.
import assert from 'node:assert/strict';
import http from 'node:http';
import { once } from 'node:events';
import { fork } from 'node:child_process';
import { randomUUID, randomBytes, createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { mkdir, writeFile, readFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
const require = createRequire(new URL('../gateway/package.json', import.meta.url));
const { WebSocket, WebSocketServer } = require('ws');
const origin = 'https://benchmark.example.test';
const secret = 'synthetic-only-secret-at-least-32-bytes';
const ids = Array.from({length:5}, (_, i) => `00000000-0000-4000-8000-${String(i+1).padStart(12,'0')}`);
const sizes = { snapshot: 32*1024, history: 128*1024, run_output: 4*1024 };
const listen = async server => { server.listen(0,'127.0.0.1'); await once(server,'listening'); return server.address().port; };
function metering() {
  const sockets = new Set(); let closedRead = 0, closedWrite = 0;
  let baseline = process.cpuUsage(), started = performance.now(), peak = process.memoryUsage().rss;
  const timer = setInterval(() => { peak = Math.max(peak, process.memoryUsage().rss); }, 10);
  timer.unref();
  return {
    track(s) { if (sockets.has(s)) return; sockets.add(s); s.once('close', () => {closedRead += s.bytesRead; closedWrite += s.bytesWritten; sockets.delete(s);}); },
    reset() { baseline = process.cpuUsage(); started = performance.now(); peak = process.memoryUsage().rss; },
    stats() { const cpu = process.cpuUsage(baseline); return {cpu_user_ms:cpu.user/1000,cpu_system_ms:cpu.system/1000,wall_ms:performance.now()-started,rss_bytes:process.memoryUsage().rss,peak_sampled_rss_bytes:peak,tcp_ingress_bytes:closedRead+[...sockets].reduce((n,s)=>n+s.bytesRead,0),tcp_egress_bytes:closedWrite+[...sockets].reduce((n,s)=>n+s.bytesWritten,0)}; }
  };
}
async function post(url, body, meter) {
  return new Promise((resolve,reject) => {
    const req = http.request(url,{method:'POST',agent:false,headers:{'content-type':'application/json',authorization:`Bearer ${secret}`}}, res => {
      let text=''; res.on('data',c=>text+=c); res.on('end',()=>{try {assert.equal(res.statusCode,200);resolve(JSON.parse(text));} catch(e){reject(e);}});
    });
    req.on('socket',s=>meter?.track(s)); req.on('error',reject); req.end(JSON.stringify(body));
  });
}
async function worker(role, config) {
  const meter = metering(); let port;
  if (role === 'vessels') {
    const server = http.createServer(); server.on('connection',s=>meter.track(s));
    const wss = new WebSocketServer({server,perMessageDeflate:false});
    wss.on('connection',(ws,req)=>{
      const vessel_id = ids[Number(req.url.split('/').pop())];
      const hello = () => ws.send(JSON.stringify({type:'hello',protocol:1,socket_id:randomUUID(),vessel_id}));
      if (req.headers.authorization) hello();
      ws.on('message',data=>{
        const frame=JSON.parse(data);
        if(frame.type==='authenticate') {hello();return;}
        const op=frame.request.command.op;
        ws.send(JSON.stringify({type:'reply',request_id:frame.request_id,response:{protocol:1,result:{kind:op,text:('synthetic '+op+' ').repeat(Math.ceil(sizes[op]/(11+op.length))).slice(0,sizes[op])},error:null,outcome_unknown:false}}));
      });
    }); port=await listen(server);
  } else if (role === 'hosted') {
    const tickets=new Map();
    const server=http.createServer(async(req,res)=>{
      let text=''; for await(const chunk of req) text+=chunk;
      const body=JSON.parse(text); let result;
      if(req.url==='/bootstrap') {
        result={vessels:ids.map((vessel_id,i)=>{
          const token=randomBytes(32).toString('base64url');
          const connection={url:`wss://v${i}.example.test/v1/vessel/socket`,token:'synthetic-vessel-token',grant_id:ids[0],vessel_id};
          tickets.set(token,{sub:createHash('sha256').update(String(body.user)).digest('hex'),tenant:ids[0],vessel:vessel_id,exp:Math.floor(Date.now()/1000)+60,connection});
          return body.mode==='legacy' ? {vessel_id,ticket:token} : {vessel_id,url:`ws://127.0.0.1:${config.vesselPort}/${i}`,ticket:token};
        })};
      } else if(req.url==='/authorize') {result=tickets.get(body.ticket);tickets.delete(body.ticket);}
      res.writeHead(result?200:403,{'content-type':'application/json'});res.end(JSON.stringify(result??{}));
    }); server.on('connection',s=>meter.track(s));port=await listen(server);
  } else {
    const {createGateway,LIMITS}=await import('../gateway/gateway.js');
    const {transport}=await import('../gateway/transport.js');
    const gateway=createGateway({secret,origin,authUrl:`http://127.0.0.1:${config.hostedPort}/authorize`},LIMITS,{
      ...transport,
      jsonPost:(url,body)=>post(url,body,meter),
      publicTarget:async url=>({url:new URL(`ws://127.0.0.1:${config.vesselPort}/${new URL(url).hostname[1]}`),options:{}}),
      openSocket:(target,connection,limits)=>{const ws=transport.openSocket(target,connection,limits);ws.on('upgrade',(_res)=>meter.track(ws._req.socket));return ws;}
    });
    gateway.server.on('connection',s=>meter.track(s));port=await listen(gateway.server);
  }
  process.on('message',msg=>{if(msg==='reset'){meter.reset();process.send({reset:true});}else if(msg==='stats')process.send({stats:meter.stats()});});
  process.send({port});
}
function inbox(ws) {
  const queue=[],waiting=[];
  ws.on('message',data=>{const frame=JSON.parse(data); if(waiting.length)waiting.shift().resolve(frame);else queue.push(frame);});
  ws.on('error',e=>{for(const w of waiting.splice(0))w.reject(e);});
  ws.on('close',()=>{for(const w of waiting.splice(0))w.reject(Error('unexpected socket close'));});
  return ()=>queue.length?Promise.resolve(queue.shift()):new Promise((resolve,reject)=>waiting.push({resolve,reject}));
}
const rpc=async(child,msg)=>{const response=once(child,'message');child.send(msg);return (await response)[0];};
async function scenario(mode,users,rounds,output,label) {
  const children=[],logs=[]; const clientMeter=metering();
  const spawn=async(role,config={})=>{
    const child=fork(fileURLToPath(import.meta.url),['--worker',role,JSON.stringify(config)],{stdio:['ignore','pipe','pipe','ipc']});children.push(child);
    child.stdout.on('data',d=>logs.push(`${role}: ${d}`));child.stderr.on('data',d=>logs.push(`${role}: ${d}`));
    const ready=await once(child,'message'); return {child,port:ready[0].port};
  };
  const timeout=setTimeout(()=>{for(const c of children)c.kill('SIGKILL');throw Error('scenario exceeded 45 seconds');},45000);
  const sockets=[];
  try {
    const vessels=await spawn('vessels');
    const hosted=await spawn('hosted',{vesselPort:vessels.port});
    const gateway=mode==='legacy'?await spawn('gateway',{vesselPort:vessels.port,hostedPort:hosted.port}):null;
    for(const child of children)await rpc(child,'reset');clientMeter.reset();
    const started=performance.now(),latencies=[];let replies=0,payloadBytes=0;
    const sessions=await Promise.all(Array.from({length:users},async(_,user)=>{
      const bootstrap=await post(`http://127.0.0.1:${hosted.port}/bootstrap`,{mode,user},clientMeter);
      return Promise.all(bootstrap.vessels.map(async(v)=>{
        const url=gateway?`ws://127.0.0.1:${gateway.port}/socket`:v.url;
        const ws=gateway?new WebSocket(url,{origin,perMessageDeflate:false}):new WebSocket(url,'voyage.vessel.v1',{origin,perMessageDeflate:false});
        sockets.push(ws);const next=inbox(ws);ws.on('upgrade',()=>clientMeter.track(ws._req.socket));await once(ws,'open');
        ws.send(JSON.stringify({type:'authenticate',ticket:v.ticket}));const hello=await next();assert.equal(hello.type,gateway?'ready':'hello');assert.equal(hello.vessel_id,v.vessel_id);
        return {ws,next};
      }));
    }));
    const bootstrap_ms=performance.now()-started;
    const bootstrapStats={};for(const [role,service]of Object.entries({vessels,hosted,gateway}))if(service)bootstrapStats[role]=(await rpc(service.child,'stats')).stats;
    const trafficStart=performance.now();
    await Promise.all(sessions.flat().map(async({ws,next})=>{
      const session_id=randomUUID(),run_id=randomUUID();
      for(let round=0;round<rounds;round++)for(const op of ['snapshot','history',...Array(8).fill('run_output')]){
        const command={op,session_id,...(op==='history'?{offset:0,limit:32}:op==='run_output'?{run_id,offset:round*32768,limit:4096}:{})};
        const request_id=randomUUID(),sent=performance.now();ws.send(JSON.stringify({type:'command',request_id,request:{protocol:1,command}}));
        const reply=await next();assert.equal(reply.request_id,request_id);assert.equal(reply.response.error,null);assert.equal(Buffer.byteLength(reply.response.result.text),sizes[op]);
        latencies.push(performance.now()-sent);replies++;payloadBytes+=sizes[op];
        // 10 requests per round per socket; paced below the unmodified 60/s gateway limit.
        await new Promise(resolve=>setTimeout(resolve,20));
      }
    }));
    const traffic_ms=performance.now()-trafficStart;const services={};
    for(const [role,service]of Object.entries({vessels,hosted,gateway}))if(service)services[role]=(await rpc(service.child,'stats')).stats;
    latencies.sort((a,b)=>a-b);
    assert.equal(replies,users*5*rounds*10);
    return {mode,users,vessels_per_user:5,rounds,replies,payload_bytes:payloadBytes,bootstrap_ms,traffic_ms,latency_ms:{p50:latencies[Math.floor(latencies.length*.50)],p95:latencies[Math.floor(latencies.length*.95)],max:latencies.at(-1)},bootstrap_services:bootstrapStats,services,client_driver:clientMeter.stats()};
  } finally {
    clearTimeout(timeout);for(const ws of sockets)ws.terminate();
    await Promise.all(children.map(async c=>{const done=once(c,'exit');c.kill();await done;}));
    await writeFile(path.join(output,`${label}.log`),logs.join(''));
  }
}
if(process.argv[2]==='--worker')await worker(process.argv[3],JSON.parse(process.argv[4]));
else {
  const output=path.resolve(process.argv[2]??'target/direct-wss-307');await mkdir(output,{recursive:true});
  const runs=[];const rounds=10;
  // Alternate ordering between independent fresh-process repetitions.
  for(const users of [1,4,10])for(let repeat=0;repeat<3;repeat++)for(const mode of repeat%2?['direct','legacy']:['legacy','direct']){
    const label=`${users}users-${repeat}-${mode}`;
    const result=await scenario(mode,users,rounds,output,label);runs.push({...result,repeat});
    await writeFile(path.join(output,`${label}.json`),JSON.stringify(result,null,2)+'\n');
    console.log(`${label}: ${result.replies} replies verified`);
  }
  const hashes={};for(const file of ['web/tests/direct-transport-benchmark.mjs','web/gateway/gateway.js','web/gateway/protocol.js','web/gateway/transport.js'])hashes[file]=createHash('sha256').update(await readFile(new URL('../../'+file,import.meta.url))).digest('hex');
  await writeFile(path.join(output,'results.json'),JSON.stringify({measured_at:new Date().toISOString(),node:process.version,ws:require('ws/package.json').version,platform:`${process.platform} ${process.arch}`,kernel:os.release(),cpu:os.cpus()[0].model,logical_cpus:os.cpus().length,sizes,rounds,hashes,runs},null,2)+'\n');
}
