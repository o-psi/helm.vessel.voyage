import test from 'node:test';
import assert from 'node:assert/strict';
import {spawn,spawnSync} from 'node:child_process';
import {mkdtempSync,mkdirSync,writeFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {randomBytes} from 'node:crypto';
import net from 'node:net';
const cwd=resolve(import.meta.dirname,'..');
async function port(){const s=net.createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p;}
test('personal tenants: HTTP session, connection isolation, direct browser credentials, deletion and logout', {timeout:60000},async()=>{
 const dir=mkdtempSync(join(tmpdir(),'helm-tenant-test-'));const database=join(dir,'db.sqlite');writeFileSync(database,'');const p=await port();
 const views=join(dir,'views');mkdirSync(views);
 const pairingSecret=randomBytes(32).toString('hex');
 const env={...process.env,APP_ENV:'local',APP_DEBUG:'false',APP_KEY:`base64:${randomBytes(32).toString('base64')}`,APP_URL:'https://helm.example',VIEW_COMPILED_PATH:views,DB_CONNECTION:'sqlite',DB_DATABASE:database,SESSION_DRIVER:'database',CACHE_STORE:'database',SESSION_SECURE_COOKIE:'false',SESSION_COOKIE:'helm_tenant_test',HELM_WEB_ENABLED:'true',HELM_WEB_GATEWAY_SECRET:'',HELM_WEB_LEGACY_GATEWAY_ENABLED:'false',GOOGLE_CLIENT_ID:'',GOOGLE_CLIENT_SECRET:'',X_CLIENT_ID:'',X_CLIENT_SECRET:'',GITHUB_CLIENT_ID:'',GITHUB_CLIENT_SECRET:''};
 const php=code=>{const r=spawnSync('php',['-r',`require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); ${code}`],{cwd,env,encoding:'utf8'});assert.equal(r.status,0,r.stderr+r.stdout);return r.stdout;};
 const migrate=spawnSync('php',['artisan','migrate','--force'],{cwd,env,encoding:'utf8'});assert.equal(migrate.status,0,migrate.stdout+migrate.stderr);
 const seed=JSON.parse(php(`
 $out=[];
 foreach(['alice','bob'] as $name){
  $tenant=App\\Models\\Tenant::create(['id'=>(string)Illuminate\\Support\\Str::uuid(),'principal_id'=>(string)Illuminate\\Support\\Str::uuid(),'name'=>$name]);
  $user=App\\Models\\User::create(['name'=>$name,'email'=>$name.'@example.test','tenant_id'=>$tenant->id,'password'=>null]);
  $connection=App\\Models\\VesselConnection::create(['tenant_id'=>$tenant->id,'name'=>$name.' vessel','endpoint'=>'https://'.$name.'.example.com','vessel_id'=>(string)Illuminate\\Support\\Str::uuid(),'credential'=>['token'=>str_repeat('a',64),'grant_id'=>(string)Illuminate\\Support\\Str::uuid()]]);
  App\\Models\\VesselPairing::create(['tenant_id'=>$tenant->id,'name'=>$name.' pending pairing','request'=>['code'=>'${pairingSecret}']]);
  $session=$app->make('session')->driver();$session->flush();$session->regenerate();Illuminate\\Support\\Facades\\Auth::login($user);$session->put('helm_operator_until',time()+3600);$session->save();
  $cookie=encrypt(Illuminate\\Cookie\\CookieValuePrefix::create('helm_tenant_test',$app['encrypter']->getKey()).$session->getId(),false);
  $out[$name]=['cookie'=>$cookie,'connection'=>$connection->id,'tenant'=>$tenant->id,'vessel_id'=>$connection->vessel_id];
 }
 echo json_encode($out);
 `));
 // Only the CLI test server installs this fake; production bootstrap stays untouched.
 // APP_ENV=local preserves real CSRF enforcement while HTTP carries fixture cookies.
 const router=join(dir,'router.php');
 writeFileSync(router,`<?php
 define('LARAVEL_START', microtime(true));
 require ${JSON.stringify(join(cwd,'vendor/autoload.php'))};
 $app=require ${JSON.stringify(join(cwd,'bootstrap/app.php'))};
 $app->instance(App\\Services\\PublicVesselHttp::class,new class extends App\\Services\\PublicVesselHttp {
  public function post(string $origin,string $path,array $body,array $headers=[]): array {
   if (!in_array($origin,['https://alice.example.com','https://bob.example.com'],true)
    || ($headers['Authorization'] ?? '') !== 'Bearer '.str_repeat('a',64)
    || !Illuminate\\Support\\Str::isUuid($headers['x-voyage-grant'] ?? '')
    || !Illuminate\\Support\\Str::isUuid($headers['x-voyage-vessel'] ?? '')) throw new RuntimeException('Unexpected fixture request');
   if ($path === '/v1/vessel/command' && $body === ['protocol'=>1,'command'=>['op'=>'capabilities']])
    return ['protocol'=>1,'error'=>null,'outcome_unknown'=>false,'result'=>['protocol'=>1,'vessel_id'=>$headers['x-voyage-vessel'],'features'=>[]]];
   if ($path !== '/v1/vessel/browser-credentials' || $body !== ['origin'=>'https://helm.example']) throw new RuntimeException('Unexpected mint request');
   return ['token'=>bin2hex(random_bytes(32)),'expires_at_ms'=>(int)floor(microtime(true)*1000)+119000,'vessel_id'=>$headers['x-voyage-vessel']];
  }
 });
 $app->handleRequest(Illuminate\\Http\\Request::capture());
 `);
 const server=spawn('php',['-S',`127.0.0.1:${p}`,'-t','public',router],{cwd,env,stdio:'ignore'});
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
  assert.equal((await call('/auth/google')).status,404);
  const a=await call('/',{},'alice');const html=await a.text();assert.equal(a.status,200);assert.match(html,/alice vessel/);assert.ok(!html.includes('bob vessel'));assert.ok(!html.includes('a'.repeat(64)));assert.ok(!html.includes(pairingSecret));
  const csrf=html.match(/name="csrf-token" content="([^"]+)"/)[1];
  const post=(path,body,who='alice',token=csrf)=>call(path,{method:'POST',headers:{Accept:'application/json','Content-Type':'application/json','X-CSRF-TOKEN':token,Origin:'https://helm.example'},body:JSON.stringify(body)},who);
  assert.equal((await post('/console/ticket',{vessel:seed.bob.connection})).status,404);
  assert.equal((await call('/connections/'+seed.bob.connection,{method:'DELETE',headers:{Accept:'application/json','X-CSRF-TOKEN':csrf}},'alice')).status,404);
  const connections=await call('/connections',{},'alice');assert.equal(connections.status,302);assert.match(connections.headers.get('location'),/manage-vessels=1/);const list=await (await call('/?manage-vessels=1',{},'alice')).text();assert.match(list,/alice vessel/);assert.ok(!list.includes('bob vessel'));assert.match(list,/data-flux-modal-trigger/);assert.match(list,/Cloudflare Tunnel/);assert.ok(list.indexOf('vessel pair-invite') > list.indexOf('<dialog'));assert.doesNotMatch(list,/<dialog[^>]*\sopen(?:\s|>)/);assert.ok(!list.includes('a'.repeat(64)));assert.match(list,/alice pending pairing/);assert.ok(!list.includes('bob pending pairing'));assert.ok(!list.includes(pairingSecret));
  const invalidPair=await call('/connections/pair',{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded','X-CSRF-TOKEN':csrf,Referer:`http://127.0.0.1:${p}/`},body:new URLSearchParams({name:'',invitation:pairingSecret})},'alice');
  assert.equal(invalidPair.status,302);
  const failedPage=await (await call('/',{},'alice')).text();assert.match(failedPage,/Check the supplied fields/);assert.match(failedPage,/\$nextTick\(\(\) => \$flux.modal/);assert.ok(!failedPage.includes(pairingSecret));
  const mintBody={vessel:seed.alice.connection};
  const mintHeaders={Accept:'application/json','Content-Type':'application/json','X-CSRF-TOKEN':csrf};
  for(const origin of [null,'https://evil.example',`http://127.0.0.1:${p}`,'https://helm.example/']) {
   const headers={...mintHeaders,...(origin === null ? {} : {Origin:origin})};
   const denied=await call('/console/ticket',{method:'POST',headers,body:JSON.stringify(mintBody)},'alice');
   assert.equal(denied.status,403,`origin ${origin}: ${await denied.text()}`);
  }
  const noCsrf=await call('/console/ticket',{method:'POST',headers:{Accept:'application/json','Content-Type':'application/json',Origin:'https://helm.example'},body:JSON.stringify(mintBody)},'alice');
  assert.equal(noCsrf.status,419);
  const issue=async(who='alice',token=csrf)=>{
   const before=Date.now();const r=await post('/console/ticket',{vessel:seed[who].connection},who,token);
   assert.equal(r.status,200,await r.clone().text());assert.match(r.headers.get('cache-control'),/no-store/);
   const text=await r.text();assert.ok(!text.includes('a'.repeat(64)));assert.ok(!text.includes(pairingSecret));
   const grant=JSON.parse(text);assert.deepEqual(Object.keys(grant).sort(),['expires_at_ms','token','url','vessel_id']);
   assert.match(grant.token,/^[a-f0-9]{64}$/);assert.equal(grant.vessel_id,seed[who].vessel_id);
   assert.equal(grant.url,`wss://${who}.example.com/v1/vessel/browser-socket`);
   assert.ok(Number.isInteger(grant.expires_at_ms));assert.ok(grant.expires_at_ms>before && grant.expires_at_ms<=Date.now()+120000);
   return grant;
  };
  const first=await issue();assert.notEqual((await issue()).token,first.token);
  // Direct credentials are not legacy gateway tickets, and no redemption service is needed.
  assert.equal((await call('/console/gateway/authorize',{method:'POST',headers:{Accept:'application/json','Content-Type':'application/json'},body:JSON.stringify({ticket:first.token})})).status,403);
  assert.equal(php("echo Illuminate\\Support\\Facades\\DB::table('web_gateway_tickets')->count();"),'0');
  assert.equal((await call('/connections/'+seed.alice.connection,{method:'DELETE',headers:{Accept:'application/json','X-CSRF-TOKEN':csrf}},'alice')).status,422);
  const deleted=await call('/connections/'+seed.alice.connection,{method:'DELETE',headers:{Accept:'application/json','Content-Type':'application/json','X-CSRF-TOKEN':csrf},body:JSON.stringify({confirm_disconnect:1})},'alice');assert.equal(deleted.status,302);
  assert.equal((await post('/console/ticket',mintBody)).status,404);
  assert.ok(!(await (await call('/?manage-vessels=1',{},'alice')).text()).includes('alice vessel'));
  // Removal/logout prevent new minting, not immediate revocation of credentials already issued.
  const bobPage=await call('/',{},'bob');const bobHtml=await bobPage.text();const bobCsrf=bobHtml.match(/name="csrf-token" content="([^"]+)"/)[1];
  await issue('bob',bobCsrf);
  const oldBobCookie=jar.bob;
  assert.equal((await post('/console/logout',{},'bob',bobCsrf)).status,302);
  // Fresh signed-out session has a new CSRF token; reach authentication rather than failing CSRF.
  const bobLogin=await call('/console/login',{},'bob');
  const loggedOutCsrf=(await bobLogin.text()).match(/name="csrf-token" content="([^"]+)"/)[1];
  assert.equal((await post('/console/ticket',{vessel:seed.bob.connection},'bob',loggedOutCsrf)).status,401);
  const replay=await call('/console/ticket',{method:'POST',headers:{...mintHeaders,'X-CSRF-TOKEN':bobCsrf,Origin:'https://helm.example',Cookie:oldBobCookie},body:JSON.stringify({vessel:seed.bob.connection})});
  assert.equal(replay.status,419);
  const logout=await post('/console/logout',{});assert.equal(logout.status,302);assert.match(logout.headers.get('location'),/landing$/);assert.equal((await call('/',{},'alice')).status,302);
  const b=await call('/',{},'bob');assert.equal(b.status,302);
 }finally{server.kill('SIGTERM');await new Promise(r=>server.once('exit',r));rmSync(dir,{recursive:true,force:true});}
});
