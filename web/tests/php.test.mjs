import test from 'node:test';
import assert from 'node:assert/strict';
import {spawn,spawnSync} from 'node:child_process';
import {mkdtempSync,writeFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {randomBytes} from 'node:crypto';
import net from 'node:net';
import {once} from 'node:events';
import {WebSocket,WebSocketServer} from '../gateway/node_modules/ws/wrapper.mjs';
import {createGateway} from '../gateway/gateway.js';
import {transport} from '../gateway/transport.js';
const cwd=resolve(import.meta.dirname,'..');
async function port(){const s=net.createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p;}
test('personal tenants: HTTP session, connection isolation, one-use tickets, deletion and logout', {timeout:60000},async()=>{
 const dir=mkdtempSync(join(tmpdir(),'helm-tenant-test-'));const database=join(dir,'db.sqlite');writeFileSync(database,'');const p=await port(),secret=randomBytes(32).toString('hex');
 const env={...process.env,APP_ENV:'local',APP_DEBUG:'false',APP_KEY:`base64:${randomBytes(32).toString('base64')}`,APP_URL:`http://127.0.0.1:${p}`,DB_CONNECTION:'sqlite',DB_DATABASE:database,SESSION_DRIVER:'database',CACHE_STORE:'database',SESSION_SECURE_COOKIE:'false',SESSION_COOKIE:'helm_tenant_test',HELM_WEB_ENABLED:'true',HELM_WEB_GATEWAY_SECRET:secret,GOOGLE_CLIENT_ID:'',GOOGLE_CLIENT_SECRET:'',X_CLIENT_ID:'',X_CLIENT_SECRET:'',GITHUB_CLIENT_ID:'',GITHUB_CLIENT_SECRET:''};
 const php=code=>{const r=spawnSync('php',['-r',`require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); ${code}`],{cwd,env,encoding:'utf8'});assert.equal(r.status,0,r.stderr+r.stdout);return r.stdout;};
 const migrate=spawnSync('php',['artisan','migrate','--force'],{cwd,env,encoding:'utf8'});assert.equal(migrate.status,0,migrate.stdout+migrate.stderr);
 const seed=JSON.parse(php(`
 $out=[];
 foreach(['alice','bob'] as $name){
  $tenant=App\\Models\\Tenant::create(['id'=>(string)Illuminate\\Support\\Str::uuid(),'principal_id'=>(string)Illuminate\\Support\\Str::uuid(),'name'=>$name]);
  $user=App\\Models\\User::create(['name'=>$name,'email'=>$name.'@example.test','tenant_id'=>$tenant->id,'password'=>null]);
  $connection=App\\Models\\VesselConnection::create(['tenant_id'=>$tenant->id,'name'=>$name.' vessel','endpoint'=>'https://'.$name.'.example.com','vessel_id'=>(string)Illuminate\\Support\\Str::uuid(),'credential'=>['token'=>str_repeat('a',64),'grant_id'=>(string)Illuminate\\Support\\Str::uuid()]]);
  $session=$app->make('session')->driver();$session->flush();$session->regenerate();Illuminate\\Support\\Facades\\Auth::login($user);$session->put('helm_operator_until',time()+3600);$session->save();
  $cookie=encrypt(Illuminate\\Cookie\\CookieValuePrefix::create('helm_tenant_test',$app['encrypter']->getKey()).$session->getId(),false);
  $out[$name]=['cookie'=>$cookie,'connection'=>$connection->id,'tenant'=>$tenant->id,'vessel_id'=>$connection->vessel_id];
 }
 echo json_encode($out);
 `));
 const server=spawn('php',['-S',`127.0.0.1:${p}`,'-t','public','public/index.php'],{cwd,env,stdio:'ignore'});
 const jar={}; const call=async(path,options={},who=null)=>{
  const cookies=who?(jar[who]||`helm_tenant_test=${encodeURIComponent(seed[who].cookie)}`):'';
  const response=await fetch(`http://127.0.0.1:${p}${path}`,{redirect:'manual',...options,headers:{Cookie:cookies,...options.headers}});
  if(who){const next=response.headers.getSetCookie().filter(c=>c.startsWith('helm_tenant_test='));if(next.length)jar[who]=next[0].split(';')[0];}return response;
 };
 try {
  for(let i=0;i<50;i++){try{await call('/up');break;}catch{await new Promise(r=>setTimeout(r,100));}}
  assert.equal((await call('/')).status,302);assert.equal((await call('/console')).status,404);
  const login=await call('/console/login');assert.equal(login.status,200);assert.match(await login.text(),/being configured/);
  assert.equal((await call('/console/login',{method:'POST'})).status,405);
  assert.equal((await call('/console/auth/google')).status,404);
  const a=await call('/',{},'alice');const html=await a.text();assert.equal(a.status,200);assert.match(html,/alice vessel/);assert.ok(!html.includes('bob vessel'));assert.ok(!html.includes('a'.repeat(64)));
  const csrf=html.match(/name="csrf-token" content="([^"]+)"/)[1];
  const post=(path,body,who='alice',token=csrf)=>call(path,{method:'POST',headers:{Accept:'application/json','Content-Type':'application/json','X-CSRF-TOKEN':token},body:JSON.stringify(body)},who);
  assert.equal((await post('/console/ticket',{vessel:seed.bob.connection})).status,404);
  assert.equal((await call('/connections/'+seed.bob.connection,{method:'DELETE',headers:{Accept:'application/json','X-CSRF-TOKEN':csrf}},'alice')).status,404);
  const connections=await call('/connections',{},'alice');const list=await connections.text();assert.match(list,/alice vessel/);assert.ok(!list.includes('bob vessel'));assert.ok(!list.includes('a'.repeat(64)));
  const issue=async()=>{const r=await post('/console/ticket',{vessel:seed.alice.connection});assert.equal(r.status,200,await r.clone().text());return (await r.json()).ticket;};
  const redeem=ticket=>call('/console/gateway/authorize',{method:'POST',headers:{Accept:'application/json','Content-Type':'application/json',Authorization:`Bearer ${secret}`},body:JSON.stringify({ticket})});
  let ticket=await issue();assert.match(ticket,/^[A-Za-z0-9_-]{43}$/);
  assert.equal((await call('/console/gateway/authorize',{method:'POST',headers:{Accept:'application/json','Content-Type':'application/json'},body:JSON.stringify({ticket})})).status,403);
  const redemption=await redeem(ticket);assert.equal(redemption.status,200,await redemption.clone().text());const grant=await redemption.json();assert.equal(grant.tenant,seed.alice.tenant);assert.equal(grant.vessel,seed.alice.connection);assert.equal(grant.connection.url,'wss://alice.example.com/v1/vessel/socket');assert.equal((await redeem(ticket)).status,403);
  // Real PHP-issued opaque ticket -> Node redemption -> fixture Vessel socket.
  const upstream=new WebSocketServer({port:0,host:'127.0.0.1',handleProtocols:()=> 'voyage.vessel.v1'});await once(upstream,'listening');
  upstream.on('connection',ws=>{ws.send(JSON.stringify({type:'hello',protocol:1,socket_id:'10000000-0000-4000-8000-000000000001',vessel_id:seed.alice.vessel_id}));ws.on('message',raw=>{const frame=JSON.parse(raw);ws.send(JSON.stringify({type:'reply',request_id:frame.request_id,response:{protocol:1,error:null,outcome_unknown:false,result:[]}}));});});
  const gateway=createGateway({secret,origin:`http://127.0.0.1:${p}`,authUrl:`http://127.0.0.1:${p}/console/gateway/authorize`},undefined,{
   ...transport,publicTarget:async url=>({url:new URL(url),options:{}}),openSocket:()=>new WebSocket(`ws://127.0.0.1:${upstream.address().port}`,'voyage.vessel.v1')
  });gateway.server.listen(0,'127.0.0.1');await once(gateway.server,'listening');
  try {
   ticket=await issue();const socket=new WebSocket(`ws://127.0.0.1:${gateway.server.address().port}/socket`,{origin:`http://127.0.0.1:${p}`});await once(socket,'open');let next=once(socket,'message');socket.send(JSON.stringify({type:'authenticate',ticket}));assert.equal(JSON.parse((await next)[0]).vessel_id,seed.alice.vessel_id);
   next=once(socket,'message');socket.send(JSON.stringify({type:'command',request_id:'10000000-0000-4000-8000-000000000002',request:{protocol:1,command:{op:'catalogue'}}}));assert.deepEqual(JSON.parse((await next)[0]).response.result,[]);socket.close();await once(socket,'close');
  }finally{await gateway.close();for(const ws of upstream.clients)ws.terminate();await new Promise(r=>upstream.close(r));}
  ticket=await issue();const deleted=await call('/connections/'+seed.alice.connection,{method:'DELETE',headers:{Accept:'application/json','X-CSRF-TOKEN':csrf}},'alice');assert.equal(deleted.status,302);assert.equal((await redeem(ticket)).status,403);
  // Logout revokes issued tickets before redemption.
  const bobPage=await call('/',{},'bob');const bobHtml=await bobPage.text();const bobCsrf=bobHtml.match(/name="csrf-token" content="([^"]+)"/)[1];
  const bobTicketResponse=await post('/console/ticket',{vessel:seed.bob.connection},'bob',bobCsrf);const bobTicket=(await bobTicketResponse.json()).ticket;
  await post('/console/logout',{},'bob',bobCsrf);assert.equal((await redeem(bobTicket)).status,403);
  const logout=await post('/console/logout',{});assert.equal(logout.status,302);assert.match(logout.headers.get('location'),/landing$/);assert.equal((await call('/',{},'alice')).status,302);
  const b=await call('/',{},'bob');assert.equal(b.status,302);
 }finally{server.kill('SIGTERM');await new Promise(r=>server.once('exit',r));rmSync(dir,{recursive:true,force:true});}
});
