import test from 'node:test';
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { randomUUID, randomBytes } from 'node:crypto';
import http from 'node:http';
import { transport } from '../transport.js';
import { WebSocket, WebSocketServer } from 'ws';
import { createGateway, verifyTicket, validateClaims, LIMITS } from '../gateway.js';
import { validateConfig, loadConfig } from '../config.js';
import { validCommand, validReply } from '../protocol.js';
const secret = 'test-only-secret-that-is-at-least-32-bytes';
const origin = 'https://helm.example.test';
const sub = 'a'.repeat(64), vesselId = randomUUID(), grantId = randomUUID();
const tenant = randomUUID(), connectionId = randomUUID(), issued = new Map();
const connection = { url:'wss://vessel.example/v1/vessel/socket', token:'private-test-token',grant_id:grantId,vessel_id:vesselId };
function ticket(extra = {}) {
  const value = randomBytes(32).toString('base64url');
  issued.set(value, { sub,tenant,vessel:connectionId,exp:Math.floor(Date.now()/1000)+60,connection,...extra }); return value;
}
function inbox(ws) {
  const queue = [], waiters = [];
  ws.on('message', data => { const f = JSON.parse(data.toString()); if (waiters.length) waiters.shift()(f); else queue.push(f); });
  return () => queue.length ? Promise.resolve(queue.shift()) : new Promise(resolve => waiters.push(resolve));
}
async function fixture(t, { limits = {}, hello = {}, onCommand, noHello = false, authOutage = false } = {}) {
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
  const auth = http.createServer(async (req,res) => {
    if (authOutage) {res.writeHead(503);res.end('private service diagnostic');return;}
    let body = ''; for await (const chunk of req) body += chunk;
    assert.equal(req.url, '/authorize'); assert.equal(req.method,'POST');
    assert.equal(req.headers.authorization, `Bearer ${secret}`);
    const {ticket} = JSON.parse(body), claims = issued.get(ticket); issued.delete(ticket);
    res.writeHead(claims ? 200 : 403, {'content-type':'application/json'}); res.end(JSON.stringify(claims ?? {}));
  });
  auth.listen(0,'127.0.0.1'); await once(auth,'listening');
  t.after(() => new Promise(resolve => auth.close(resolve)));
  const config = { secret, origin, authUrl:`http://127.0.0.1:${auth.address().port}/authorize` };
  // Only module tests inject the fixture transport; production has no private bypass.
  const io = { ...transport, publicTarget: async () => ({url:new URL(`ws://127.0.0.1:${upstream.address().port}/v1/vessel/socket`), options:{}}) };
  const gateway = createGateway(config, { ...LIMITS, ...limits }, io);
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

test('opaque ticket and exact redeemed identity/lifetime', () => {
  const value = ticket(); assert.equal(verifyTicket(value),value);
  for (const v of ['bad', `${value}=`, 'x'.repeat(513)]) assert.throws(() => verifyTicket(v));
  const now = Date.now(), claims = {...issued.get(value),exp:Math.floor(now/1000)+60}; assert.equal(validateClaims(claims,now).sub,sub);
  for (const extra of [{exp:Math.floor(now/1000)-1},{exp:Math.floor(now/1000)+61},{tenant:'bad'},{sub:'bad'},{extra:true}]) assert.throws(() => validateClaims({...claims,...extra},now));
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

test('production config only supports fixed loopback authorization and local listener', async () => {
  const c = {secret,origin,authUrl:'http://127.0.0.1/authorize'};
  validateConfig(c);
  for (const extra of [{authUrl:'http://localhost/authorize'},{authUrl:'https://127.0.0.1/authorize'}, {authUrl:'http://example.org/authorize'}, {host:'0.0.0.0'}, {allowLoopback:true},{vessels:{}},{secret:'short'}]) assert.throws(() => validateConfig({...c,...extra}));
  await assert.rejects(loadConfig({HELM_WEB_ALLOW_LOOPBACK_WS:'1'}));
});

test('capabilities projection strips unallowlisted features and metadata', async t => {
  const f = await fixture(t,{onCommand(ws,frame) {
    ws.send(JSON.stringify({type:'reply',request_id:frame.request_id,response:{protocol:1,error:null,outcome_unknown:false,result:{protocol:1,vessel_id:vesselId,version:'test',scope:'session',session_id:randomUUID(),features:['duplex_socket','scoped_catalogue','sse_events','browser','start_settings','notifications','voyage_operations'],rights:['execute'],token:'never forward',future:{execution:true}}}}));
  }});
  const {ws,next} = await f.connect(); await next(); ws.send(JSON.stringify(command({op:'capabilities'})));
  const {response} = await next(); assert.deepEqual(response.result.features,['duplex_socket','scoped_catalogue']);
  for (const key of ['rights','token','future']) assert.equal(Object.hasOwn(response.result,key),false);
});

test('renewal refuses tenant, connection and credential changes', async t => {
  const f = await fixture(t);
  for (const change of [{tenant:randomUUID()},{vessel:randomUUID()},{connection:{...connection,token:'changed'}},{connection:{...connection,grant_id:randomUUID()}},{connection:{...connection,url:'wss://other.example/v1/vessel/socket'}},{connection:{...connection,vessel_id:randomUUID()}}]) {
    const b = await f.connect(); await b.next(); const closed = once(b.ws,'close');
    b.ws.send(JSON.stringify({type:'authenticate',ticket:ticket(change)})); assert.equal((await closed)[0],1008);
  }
});

test('local secret endpoints pair exact protocol and probe only filtered capabilities', async t => {
  const f = await fixture(t,{onCommand(ws,frame) {
    assert.equal(frame.request.command.op,'capabilities');
    ws.send(JSON.stringify({type:'reply',request_id:frame.request_id,response:{protocol:1,error:null,outcome_unknown:false,result:{protocol:1,vessel_id:vesselId,features:['duplex_socket','browser'],token:'do-not-return'}}}));
  }});
  const url = f.url.replace('ws:','http:').replace('/socket','/probe');
  const response = await fetch(url,{method:'POST',headers:{authorization:`Bearer ${secret}`,'content-type':'application/json'},body:JSON.stringify({connection})});
  assert.equal(response.status,200);assert.deepEqual(await response.json(),{protocol:1,vessel_id:vesselId,features:['duplex_socket']});
  for (const [path,headers,body,status] of [['/probe',{}, {connection},403],['/probe',{origin}, {connection},403],['/probe?x=1',{}, {},404],['/probe',{}, {connection,command:'catalogue'},502]]) {
    const res = await fetch(url.replace('/probe',path),{method:'POST',headers:{authorization:`Bearer ${secret}`,'content-type':'application/json',...headers,...(status===403&&!headers.origin?{authorization:'Bearer wrong'}:{})},body:JSON.stringify(body)});
    assert.equal(res.status,status);await res.text();
  }
});

test('pair forwards exact HTTP request and never retries uncertain result', async t => {
  const calls = [];
  const io = {...transport, publicTarget:async (url,pair) => {assert.equal(pair,true);assert.equal(url,'https://vessel.example');return {url:new URL(url),options:{}};},jsonPost:async (...args) => {calls.push(args);return {protocol:1,result:{token:'fixture'},error:null,outcome_unknown:false};}};
  const gateway = createGateway({secret,origin,authUrl:'http://127.0.0.1/authorize'},LIMITS,io);
  gateway.server.listen(0,'127.0.0.1');await once(gateway.server,'listening');t.after(()=>gateway.close());
  const value = {endpoint:'https://vessel.example',principal_id:randomUUID(),invitation_id:randomUUID(),command_id:randomUUID(),code:'private-code',vessel_id:vesselId};
  const res = await fetch(`http://127.0.0.1:${gateway.server.address().port}/pair`,{method:'POST',headers:{authorization:`Bearer ${secret}`,'content-type':'application/json'},body:JSON.stringify(value)});
  assert.equal(res.status,200);assert.equal((await res.json()).result.token,'fixture');assert.equal(calls.length,1);
  assert.equal(calls[0][0].href,'https://vessel.example/v1/vessel/pair');
  assert.deepEqual(calls[0][1],{protocol:1,principal_id:value.principal_id,invitation_id:value.invitation_id,command_id:value.command_id,code:value.code});
  assert.equal(calls[0][2].headers['x-voyage-vessel'],vesselId);
  io.jsonPost = async () => ({protocol:1,result:null,error:'private upstream diagnostic',outcome_unknown:true});
  const refused = await fetch(`http://127.0.0.1:${gateway.server.address().port}/pair`,{method:'POST',headers:{authorization:`Bearer ${secret}`,'content-type':'application/json'},body:JSON.stringify(value)});
  assert.equal(refused.status,502);assert.deepEqual(await refused.json(),{error:'gateway refused'});
});

test('redemption outage fails closed without opening upstream', async t => {
  const f = await fixture(t,{authOutage:true}); const {ws} = await f.connect();
  const [code,reason] = await once(ws,'close');assert.equal(code,1008);assert.equal(reason.toString(),'gateway refused');assert.equal(f.requests.length,0);
});
