import test from 'node:test';
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { randomUUID, createHmac } from 'node:crypto';
import { mkdtemp, writeFile, chmod, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { WebSocket, WebSocketServer } from 'ws';
import { createGateway, verifyTicket, LIMITS } from '../gateway.js';
import { validateConfig, loadConfig } from '../config.js';
import { validCommand, validReply } from '../protocol.js';
const secret = 'test-only-secret-that-is-at-least-32-bytes';
const origin = 'https://helm.example.test';
const sub = 'a'.repeat(64), vesselId = randomUUID(), grantId = randomUUID();
function ticket(extra = {}, key = secret) {
  const p = Buffer.from(JSON.stringify({ aud: 'helm-web-gateway', sub, jti: randomUUID(), exp: Math.floor(Date.now()/1000)+60, vessel: 'test', ...extra })).toString('base64url');
  return `${p}.${createHmac('sha256', key).update(p).digest('base64url')}`;
}
function inbox(ws) {
  const queue = [], waiters = [];
  ws.on('message', data => { const f = JSON.parse(data.toString()); if (waiters.length) waiters.shift()(f); else queue.push(f); });
  return () => queue.length ? Promise.resolve(queue.shift()) : new Promise(resolve => waiters.push(resolve));
}
async function fixture(t, { limits = {}, hello = {}, onCommand, noHello = false } = {}) {
  const upstream = new WebSocketServer({ port: 0, host: '127.0.0.1', handleProtocols: protocols => protocols.has('voyage.vessel.v1') && 'voyage.vessel.v1' });
  await once(upstream, 'listening');
  const requests = [], sockets = [];
  upstream.on('connection', (ws, req) => {
    requests.push(req); sockets.push(ws);
    if (!noHello) ws.send(JSON.stringify({ type: 'hello', protocol: 1, socket_id: randomUUID(), vessel_id: vesselId, ...hello }));
    ws.on('message', data => {
      const f = JSON.parse(data.toString());
      if (onCommand) onCommand(ws, f);
      else ws.send(JSON.stringify({ type: 'reply', request_id: f.request_id, response: { protocol: 1, result: { op: f.request.command.op }, error: null, outcome_unknown: false } }));
    });
  });
  const config = { secret, origin, allowLoopback: true, vessels: { test: { url: `ws://127.0.0.1:${upstream.address().port}/v1/vessel/socket`, token: 'private-test-token', grant_id: grantId, vessel_id: vesselId } } };
  const gateway = createGateway(config, { ...LIMITS, ...limits });
  gateway.server.listen(0, '127.0.0.1'); await once(gateway.server, 'listening');
  const url = `ws://127.0.0.1:${gateway.server.address().port}/socket`;
  t.after(async () => { await gateway.close(); for (const ws of upstream.clients) ws.terminate(); await new Promise(resolve => upstream.close(resolve)); });
  async function connect(auth = ticket()) {
    const ws = new WebSocket(url, { origin }); const next = inbox(ws);
    await once(ws, 'open');
    if (auth) ws.send(JSON.stringify({ type: 'authenticate', ticket: auth }));
    return { ws, next };
  }
  return { url, connect, requests, sockets, config };
}
const command = c => ({ type: 'command', request_id: randomUUID(), request: { protocol: 1, command: c } });

test('ticket canonical signature, exact claims, identity and lifetime', () => {
  assert.equal(verifyTicket(ticket(), secret).sub, sub);
  for (const value of [ticket({}, 'wrong'), ticket({ exp: Math.floor(Date.now()/1000)-1 }), ticket({ exp: Math.floor(Date.now()/1000)+61 }), ticket({ aud: 'other' }), ticket({ jti: 'no' }), ticket({ sub: 'session' }), ticket({ extra: true }), 'bad', `${ticket()}=`]) assert.throws(() => verifyTicket(value, secret));
});

test('strict public operation shapes and forbidden execution surfaces', () => {
  const session_id = randomUUID(), incarnation = randomUUID(), run_id = randomUUID();
  const mutation = { command_id: randomUUID(), expected_revision: 1, expires_at_ms: Date.now()+10000 };
  const allowed = [{op:'capabilities'}, {op:'catalogue'}, {op:'inspect', session_id}, {op:'snapshot',session_id}, {op:'decisions',session_id},
    {op:'history',session_id,offset:0,limit:10,expected_revision:null}, {op:'message_chunk',session_id,index:0,offset:0,limit:4096,expected_revision:1},
    {op:'run_output',session_id,run_id,offset:0,limit:4096}, {op:'receipt',session_id,command_id:randomUUID()},
    {op:'events',session_id,incarnation,after:0,limit:10,wait_ms:100}, {op:'submit',session_id,...mutation,prompt:'hello'},
    {op:'steer',session_id,incarnation,run_id,...mutation,prompt:'hello'}, {op:'cancel',session_id,incarnation,run_id,...mutation},
    {op:'respond',session_id,incarnation,run_id,...mutation,decision_id:randomUUID(),response:{answer:'yes'}}];
  for (const c of allowed) assert.ok(validCommand(command(c)), c.op);
  for (const op of ['browser','execute_tool','terminal','configure','grant','start','resolve','notifications','__proto__']) assert.equal(validCommand(command({op,session_id})), false);
  assert.equal(validCommand(command({op:'capabilities',token:'leak'})),false);
  assert.equal(validCommand(command({op:'cancel',session_id,run_id,...mutation})),false);
  assert.equal(validCommand(command({op:'history',session_id,offset:0,limit:129})),false);
  assert.equal(validCommand(command({op:'events',session_id,after:0,limit:1,wait_ms:30000})),false);
  assert.equal(validReply({type:'reverse_request',request_id:randomUUID(),request:{}}),false);
});

test('upstream journey: pinned hello, headers, command/reply, refusal, renewal and no replay', async t => {
  let commands = 0;
  const f = await fixture(t, { onCommand(ws, frame) { commands++; ws.send(JSON.stringify({type:'reply',request_id:frame.request_id,response:{protocol:1,result:null,error:'scope refused',outcome_unknown:false}})); } });
  const auth = ticket(); const {ws,next} = await f.connect(auth);
  assert.deepEqual(await next(),{type:'ready',vessel_id:vesselId});
  assert.equal(f.requests[0].headers.authorization,'Bearer private-test-token');
  assert.equal(f.requests[0].headers['x-voyage-grant'],grantId);
  assert.equal(f.requests[0].headers['x-voyage-vessel'],vesselId);
  assert.equal(f.requests[0].headers['sec-websocket-protocol'],'voyage.vessel.v1');
  const frame = command({op:'catalogue'}); ws.send(JSON.stringify(frame));
  const reply = await next(); assert.equal(reply.request_id,frame.request_id); assert.equal(reply.response.error,'scope refused');
  ws.send(JSON.stringify({type:'authenticate',ticket:ticket()})); assert.deepEqual(await next(),{type:'ready',vessel_id:vesselId}); assert.equal(f.requests.length,1);
  const second = await f.connect(auth); assert.equal((await once(second.ws,'close'))[0],1008);
  const closed = once(ws,'close'); ws.send(JSON.stringify(frame)); assert.equal((await closed)[0],1008); assert.equal(commands,1);
});

test('upgrade rejects missing/wrong origin and every query token', async t => {
  const f = await fixture(t);
  for (const [url, options] of [[f.url,{}],[f.url,{origin:'https://other.test'}],[`${f.url}?ticket=secret`,{origin}]]) {
    const ws = new WebSocket(url,options);
    ws.on('error',()=>{});
    const [req,res] = await once(ws,'unexpected-response'); assert.equal(res.statusCode,403); res.resume(); req.destroy();
  }
  assert.equal(f.requests.length,0);
});

test('auth deadline, non-auth first frame, binary, malformed JSON and forbidden command', async t => {
  const f = await fixture(t,{limits:{authMs:40}});
  const unauth = await f.connect(null); assert.equal((await once(unauth.ws,'close'))[0],1008);
  for (const payload of [JSON.stringify(command({op:'catalogue'})), '{', Buffer.from('{}')]) {
    const {ws} = await f.connect(null); const closed = once(ws,'close'); ws.send(payload); assert.equal((await closed)[0],1008);
  }
  const {ws,next} = await f.connect(); await next(); const closed = once(ws,'close'); ws.send(JSON.stringify(command({op:'terminal',session_id:randomUUID()}))); assert.equal((await closed)[0],1008);
});

test('wrong hello, hello timeout, reverse requests and unmatched replies fail closed', async t => {
  for (const opts of [{hello:{vessel_id:randomUUID()}},{hello:{protocol:2}},{noHello:true,limits:{helloMs:40}}]) {
    const f = await fixture(t,opts); const {ws} = await f.connect(); await once(ws,'close');
  }
  for (const frame of [{type:'reverse_request',request_id:randomUUID(),request:{kind:'browser_work'}}, {type:'reply',request_id:randomUUID(),response:{protocol:1,result:null,error:null,outcome_unknown:false}}]) {
    const f = await fixture(t); const {ws,next} = await f.connect(); await next(); const closed = once(ws,'close'); f.sockets[0].send(JSON.stringify(frame)); assert.equal((await closed)[0],1008);
  }
});

test('lease expiry closes both; changed renewal identity is refused', async t => {
  const f = await fixture(t);
  const {ws,next} = await f.connect(ticket({exp:Math.floor(Date.now()/1000)+1})); await next();
  const upstreamClosed = once(f.sockets[0],'close'); assert.equal((await once(ws,'close'))[0],1008); await upstreamClosed;
  const b = await f.connect(); await b.next(); const closed = once(b.ws,'close'); b.ws.send(JSON.stringify({type:'authenticate',ticket:ticket({sub:'b'.repeat(64)})})); assert.equal((await closed)[0],1008);
});

test('bounded pending timeout, inflight, rate, payload, subject and global connections', async t => {
  for (const opts of [{limits:{requestMs:40},onCommand(){}},{limits:{inflight:1},onCommand(){}},{limits:{rate:2}}]) {
    const f = await fixture(t,opts); const {ws,next} = await f.connect(); await next(); const closed = once(ws,'close');
    ws.send(JSON.stringify(command({op:'catalogue'})));
    if (!opts.limits.requestMs) ws.send(JSON.stringify(command({op:'catalogue'})));
    await closed;
  }
  const f = await fixture(t,{limits:{bytes:1024,perSubject:1,connections:2}});
  const a = await f.connect(); await a.next(); const b = await f.connect(); assert.equal((await once(b.ws,'close'))[0],1008);
  const closed = once(a.ws,'close'); a.ws.send('x'.repeat(1025)); assert.equal((await closed)[0],1009);
  const g = await fixture(t,{limits:{connections:1}}); const c = await g.connect(); await c.next();
  const ws = new WebSocket(g.url,{origin}); ws.on('error',()=>{}); const [req,res] = await once(ws,'unexpected-response'); assert.equal(res.statusCode,403); res.resume(); req.destroy();
});

test('heartbeat closes a browser that does not pong', async t => {
  const f = await fixture(t,{limits:{heartbeatMs:30}});
  const ws = new WebSocket(f.url,{origin,autoPong:false}); const next = inbox(ws); await once(ws,'open'); ws.send(JSON.stringify({type:'authenticate',ticket:ticket()})); await next(); assert.equal((await once(ws,'close'))[0],1011);
});

test('private config file and secure upstream URL restrictions', async t => {
  const f = await fixture(t);
  for (const url of ['ws://example.test/v1/vessel/socket','ws://localhost/v1/vessel/socket','wss://example.test/other','wss://user:pass@example.test/v1/vessel/socket','wss://example.test/v1/vessel/socket?q=x']) assert.throws(()=>validateConfig({...f.config,vessels:{test:{...f.config.vessels.test,url}}}));
  assert.throws(()=>validateConfig({...f.config,allowLoopback:false}));
  assert.throws(()=>validateConfig({...f.config,secret:'short'}));
  const dir = await mkdtemp(join(tmpdir(),'helm-gateway-test-')); t.after(()=>rm(dir,{recursive:true,force:true}));
  const path = join(dir,'vessels.json'); await writeFile(path,JSON.stringify(f.config.vessels),{mode:0o600});
  const env = {HELM_WEB_GATEWAY_SECRET:secret,HELM_WEB_ORIGIN:origin,HELM_WEB_VESSELS_FILE:path,HELM_WEB_ALLOW_LOOPBACK_WS:'1'};
  assert.equal((await loadConfig(env)).port,8787);
  await chmod(path,0o644); await assert.rejects(loadConfig(env));
});

test('capabilities projection strips unallowlisted features and metadata', async t => {
  const f = await fixture(t,{onCommand(ws,frame) {
    ws.send(JSON.stringify({type:'reply',request_id:frame.request_id,response:{protocol:1,error:null,outcome_unknown:false,result:{protocol:1,vessel_id:vesselId,version:'test',scope:'session',session_id:randomUUID(),features:['duplex_socket','scoped_catalogue','sse_events','browser','start_settings','notifications','voyage_operations'],rights:['execute'],token:'never forward',future:{execution:true}}}}));
  }});
  const {ws,next} = await f.connect(); await next(); ws.send(JSON.stringify(command({op:'capabilities'})));
  const {response} = await next(); assert.deepEqual(response.result.features,['duplex_socket','scoped_catalogue']);
  for (const key of ['rights','token','future']) assert.equal(Object.hasOwn(response.result,key),false);
});

test('startup replay blackout refuses admission before ticket horizon', async t => {
  const f = await fixture(t); f.config.admissionNotBefore = Date.now()+60000;
  const ws = new WebSocket(f.url,{origin}); ws.on('error',()=>{});
  const [req,res] = await once(ws,'unexpected-response'); assert.equal(res.statusCode,403); res.resume(); req.destroy();
  assert.equal(f.requests.length,0);
});
