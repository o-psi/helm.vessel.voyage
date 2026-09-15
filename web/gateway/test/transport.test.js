import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import https from 'node:https';
import { once } from 'node:events';
import { execFileSync } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { WebSocketServer } from 'ws';
import { randomUUID } from 'node:crypto';
import { isPublicAddress, publicTarget, endpoint, jsonPost, openSocket } from '../transport.js';
import { LIMITS } from '../gateway.js';

test('deny special-use IPv4 and IPv6, including mapped, NAT64, multicast and transition ranges', () => {
  for (const ip of ['0.0.0.0','10.1.2.3','100.64.0.1','127.0.0.1','169.254.169.254','172.31.1.1','192.168.1.1','192.0.0.9','192.0.2.1','192.88.99.1','198.19.0.1','198.51.100.1','203.0.113.1','224.0.0.1','255.255.255.255','::','::1','::ffff:8.8.8.8','64:ff9b::808:808','fc00::1','fe80::1','ff02::1','2001::1','2001:db8::1','2002:0808:0808::1','3fff::1','fe80::1%eth0','bad']) assert.equal(isPublicAddress(ip),false,ip);
  for (const ip of ['8.8.8.8','1.1.1.1','2606:4700:4700::1111','2001:4860:4860::8888']) assert.equal(isPublicAddress(ip),true,ip);
});
test('canonical endpoint and all-address validation with single pinned lookup', async () => {
  for (const url of ['ws://example.org/v1/vessel/socket','wss://example.org:8443/v1/vessel/socket','wss://user@example.org/v1/vessel/socket','wss://example.org/v1/vessel/socket?q=1','wss://example.org/other','wss://example.org/a/../v1/vessel/socket']) assert.throws(() => endpoint(url));
  await assert.rejects(publicTarget('wss://127.0.0.1/v1/vessel/socket'));
  await assert.rejects(publicTarget('wss://vessel.example/v1/vessel/socket', false, async () => [{address:'8.8.8.8',family:4},{address:'::1',family:6}]));
  let lookups = 0;
  const target = await publicTarget('wss://vessel.example/v1/vessel/socket',false,async () => { lookups++; return [{address:'8.8.8.8',family:4}]; });
  assert.equal(target.options.servername,'vessel.example'); assert.equal(target.options.rejectUnauthorized,true); assert.equal(target.options.agent,false);
  for (let i=0;i<2;i++) target.options.lookup('vessel.example',{},(err,address,family) => {assert.equal(err,null);assert.equal(address,'8.8.8.8');assert.equal(family,4);});
  assert.equal(lookups,1);
});
test('HTTP redemption refuses redirect, oversized response, malformed JSON and deadline', async t => {
  const server = http.createServer((req,res) => {
    res.setHeader('content-type','application/json');
    if (req.url === '/redirect') {res.writeHead(302,{location:'/ok'});res.end('{}');}
    else if (req.url === '/large') res.end('x'.repeat(100));
    else if (req.url === '/bad') res.end('{');
    else if (req.url === '/slow') {} else res.end('{}');
  });
  server.listen(0,'127.0.0.1'); await once(server,'listening');
  t.after(() => {server.closeAllConnections();return new Promise(r => server.close(r));});
  for (const path of ['/redirect','/large','/bad','/slow']) await assert.rejects(jsonPost(new URL(`http://127.0.0.1:${server.address().port}${path}`),{ticket:'private'},{maxBytes:32,timeout:30}));
});
test('real TLS websocket fixture preserves hostname verification and pinned lookup', async t => {
  const dir = await mkdtemp(join(tmpdir(),'gateway-tls-')); t.after(() => rm(dir,{recursive:true,force:true}));
  execFileSync('openssl',['req','-x509','-newkey','rsa:2048','-nodes','-keyout',join(dir,'key'),'-out',join(dir,'cert'),'-days','1','-subj','/CN=vessel.example','-addext','subjectAltName=DNS:vessel.example'],{stdio:'ignore'});
  const cert = await readFile(join(dir,'cert'));
  const server = https.createServer({key:await readFile(join(dir,'key')),cert});
  const wss = new WebSocketServer({server,handleProtocols:()=>'voyage.vessel.v1'});
  wss.on('connection',ws => ws.send('{}'));
  server.listen(0,'127.0.0.1');await once(server,'listening');
  t.after(async () => {for (const ws of wss.clients) ws.terminate();await new Promise(r=>wss.close(r));await new Promise(r=>server.close(r));});
  let called = 0;
  const options = {ca:cert,rejectUnauthorized:true,agent:false,lookup:(host,opts,cb) => {called++;assert.equal(host,'vessel.example');opts.all?cb(null,[{address:'127.0.0.1',family:4}]):cb(null,'127.0.0.1',4);}};
  const v = {token:'test-token',grant_id:randomUUID(),vessel_id:randomUUID()};
  const target = {url:new URL(`wss://vessel.example:${server.address().port}/v1/vessel/socket`),options};
  const ws = openSocket(target,v,LIMITS); await once(ws,'open');assert.equal(called,1);ws.terminate();await once(ws,'close');
  const bad = openSocket({...target,options:{...options,servername:'wrong.example'}},v,LIMITS);
  const [error] = await once(bad,'error');assert.equal(error.code,'ERR_TLS_CERT_ALTNAME_INVALID');
});
