// Actual HTTP middleware/session/CSRF journey, with an isolated disposable SQLite database.
import test from 'node:test';
import assert from 'node:assert/strict';
import {spawn, spawnSync} from 'node:child_process';
import {mkdtempSync, writeFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {randomBytes, createHmac} from 'node:crypto';
import net from 'node:net';
import {once} from 'node:events';
import {WebSocket,WebSocketServer} from '../gateway/node_modules/ws/wrapper.mjs';
import {createGateway} from '../gateway/gateway.js';
const cwd = resolve(import.meta.dirname,'..');
async function port() { const server = net.createServer(); await new Promise(r=>server.listen(0,'127.0.0.1',r)); const value=server.address().port; await new Promise(r=>server.close(r)); return value; }
test('Laravel console: fail closed, CSRF, login, ticket scope, logout and throttling', {timeout:60000}, async () => {
    const dir=mkdtempSync(join(tmpdir(),'helm-web-test-')), database=join(dir,'database.sqlite'); writeFileSync(database,'');
    const password='synthetic-test-password';
    const hash=spawnSync('php',['-r',`echo password_hash('${password}', PASSWORD_DEFAULT);`],{encoding:'utf8'}).stdout;
    const secret=randomBytes(32).toString('hex'), p=await port();
    const env={...process.env,APP_ENV:'local',APP_DEBUG:'false',APP_KEY:`base64:${randomBytes(32).toString('base64')}`,APP_URL:`http://127.0.0.1:${p}`,DB_CONNECTION:'sqlite',DB_DATABASE:database,SESSION_DRIVER:'database',CACHE_STORE:'database',SESSION_SECURE_COOKIE:'false',SESSION_COOKIE:'helm_console_test',HELM_WEB_ENABLED:'true',HELM_WEB_PASSWORD_HASH:hash,HELM_WEB_GATEWAY_SECRET:secret,HELM_WEB_VESSELS:'local'};
    const migrate=spawnSync('php',['artisan','migrate','--force'],{cwd,env,encoding:'utf8'}); assert.equal(migrate.status,0,migrate.stderr);
    const server=spawn('php',['-S',`127.0.0.1:${p}`,'-t','public','public/index.php'],{cwd,env,stdio:'ignore'});
    let cookies = new Map();
    const call=async(path,options={})=>{const response=await fetch(`http://127.0.0.1:${p}${path}`,{redirect:'manual',...options,headers:{Cookie:[...cookies].map(([k,v])=>`${k}=${v}`).join('; '),...options.headers}}); for(const cookie of response.headers.getSetCookie()){const pair=cookie.split(';')[0],i=pair.indexOf('=');cookies.set(pair.slice(0,i),pair.slice(i+1));} return response;};
    try {
        for(let i=0;i<60;i++){try{await call('/up');break;}catch{await new Promise(r=>setTimeout(r,100));}}
        assert.equal((await call('/console')).status,302);
        const anonymous=await call('/'); assert.equal(anonymous.status,302); assert.match(anonymous.headers.get('location'),/\/landing$/);
        const landing=await call('/landing'); assert.equal(landing.status,200); assert.match(await landing.text(),/Sign in/);
        let response=await call('/console/ticket',{method:'POST',headers:{Accept:'application/json','Content-Type':'application/json'},body:JSON.stringify({vessel:'local'})}); assert.equal(response.status,419);
        const login=await call('/console/login'), html=await login.text(); assert.equal(login.status,200); assert.match(login.headers.get('cache-control'),/no-store/); assert.equal(login.headers.get('x-frame-options'),'DENY');
        const csrf=html.match(/name="csrf-token" content="([^"]+)"/)[1];
        const post=(path,values,token=csrf)=>call(path,{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded','X-CSRF-TOKEN':token},body:new URLSearchParams(values)});
        response=await post('/console/login',{password:'wrong'}); assert.equal(response.status,302);
        response=await post('/console/login',{password}); assert.equal(response.status,302); assert.equal(new URL(response.headers.get('location')).pathname,'/');
        const consolePage=await call('/'); const consoleHtml=await consolePage.text(); assert.equal(consolePage.status,200); assert.match(consoleHtml,/id="helm-client"/); assert.ok(!consoleHtml.includes(secret)); assert.ok(!consoleHtml.includes(hash));
        const token=consoleHtml.match(/name="csrf-token" content="([^"]+)"/)[1];
        const getTicket=vessel=>call('/console/ticket',{method:'POST',headers:{Accept:'application/json','Content-Type':'application/json','X-CSRF-TOKEN':token},body:JSON.stringify({vessel})});
        assert.equal((await getTicket('unconfigured')).status,422);
        response=await getTicket('local'); assert.equal(response.status,200);
        const [payload,signature]=(await response.json()).ticket.split('.');
        assert.equal(signature,createHmac('sha256',secret).update(payload).digest('base64url'));
        const claims=JSON.parse(Buffer.from(payload,'base64url')); assert.equal(claims.aud,'helm-web-gateway'); assert.equal(claims.vessel,'local'); assert.ok(claims.exp*1000<=Date.now()+60000); assert.ok(claims.exp*1000>Date.now());
        // Real Laravel-issued ticket crosses the actual gateway into a fixture Vessel.
        const vesselId='10000000-0000-4000-8000-000000000001';
        const upstream=new WebSocketServer({port:0,host:'127.0.0.1',handleProtocols:()=> 'voyage.vessel.v1'}); await once(upstream,'listening');
        upstream.on('connection',(ws,req)=>{
            assert.equal(req.headers['x-voyage-vessel'],vesselId);
            ws.send(JSON.stringify({type:'hello',protocol:1,socket_id:'10000000-0000-4000-8000-000000000002',vessel_id:vesselId}));
            ws.on('message',raw=>{const frame=JSON.parse(raw);ws.send(JSON.stringify({type:'reply',request_id:frame.request_id,response:{protocol:1,error:null,outcome_unknown:false,result:[]}}));});
        });
        const gateway=createGateway({secret,origin:`http://127.0.0.1:${p}`,allowLoopback:true,vessels:{local:{url:`ws://127.0.0.1:${upstream.address().port}/v1/vessel/socket`,token:'synthetic-scoped-token',grant_id:'10000000-0000-4000-8000-000000000003',vessel_id:vesselId}}});
        gateway.server.listen(0,'127.0.0.1'); await once(gateway.server,'listening');
        try {
            const socket=new WebSocket(`ws://127.0.0.1:${gateway.server.address().port}/socket`,{origin:`http://127.0.0.1:${p}`});await once(socket,'open');
            let next=once(socket,'message');socket.send(JSON.stringify({type:'authenticate',ticket:`${payload}.${signature}`}));
            assert.equal(JSON.parse((await next)[0]).vessel_id,vesselId);
            next=once(socket,'message');socket.send(JSON.stringify({type:'command',request_id:'10000000-0000-4000-8000-000000000004',request:{protocol:1,command:{op:'catalogue'}}}));
            assert.deepEqual(JSON.parse((await next)[0]).response.result,[]);socket.close();await once(socket,'close');
        } finally {await gateway.close();for(const ws of upstream.clients)ws.terminate();await new Promise(r=>upstream.close(r));}
        response=await post('/console/logout',{},token); assert.equal(response.status,302); assert.match(response.headers.get('location'),/\/landing$/);
        const again=await call('/console/login'), againHtml=await again.text(), loggedOutToken=againHtml.match(/name="csrf-token" content="([^"]+)"/)[1];
        response=await call('/console/ticket',{method:'POST',headers:{Accept:'application/json','Content-Type':'application/json','X-CSRF-TOKEN':loggedOutToken},body:'{"vessel":"local"}'}); assert.equal(response.status,401);
        for(let i=0;i<5;i++) assert.equal((await post('/console/login',{password:'wrong'},loggedOutToken)).status,302);
        assert.equal((await post('/console/login',{password:'wrong'},loggedOutToken)).status,429);
        assert.equal((await call('/')).status,302); assert.equal((await call('/landing')).status,200);
    } finally {server.kill('SIGTERM');await new Promise(r=>server.once('exit',r));rmSync(dir,{recursive:true,force:true});}
    // Test disabled deployment via a separate process; never enable a real deployment.
    const disabled=spawnSync('php',['-r',`require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $kernel=$app->make(Illuminate\\Contracts\\Http\\Kernel::class); $r=$kernel->handle(Illuminate\\Http\\Request::create('http://localhost/console/login')); echo $r->getStatusCode();`],{cwd,env:{...env,HELM_WEB_ENABLED:'false',SESSION_DRIVER:'array'},encoding:'utf8'});
    assert.equal(disabled.status,0); assert.equal(disabled.stdout,'404');
});
