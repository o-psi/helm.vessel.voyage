// Scoped synthetic evidence for this adapter. No provider, Vessel or general test harness.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {chromium} from 'playwright-core';
const here=path.dirname(fileURLToPath(import.meta.url));
const root=await fs.mkdtemp(path.join(process.env.HELM_BROWSER_PROBE_ROOT||os.tmpdir(),'helm-browser-probe-'));
await fs.chmod(root,0o700);
const fixtures=http.createServer((req,res)=> {
  if(req.url==='/redirect'){res.writeHead(302,{Location:'http://localhost:1/forbidden'}).end();return;}
  if(req.url==='/file'){res.writeHead(200,{'Content-Disposition':'attachment; filename="synthetic.txt"','Content-Type':'application/octet-stream'}).end('synthetic local download');return;}
  res.setHeader('Content-Type','text/html');res.end('<!doctype html><title>Local fixture</title><h1>Synthetic page</h1><button id="counter" onclick="this.textContent=\'Clicked\'">Count</button><input id="text" aria-label="Fixture input"><input type="password" value="private-password-marker"><a href="/file">Download fixture</a><input id="file" type="file">');
});
await new Promise(r=>fixtures.listen(0,'127.0.0.1',r));
const fixture=`http://127.0.0.1:${fixtures.address().port}`;
const helper=spawn(process.execPath,[path.join(here,'helper.mjs')],{stdio:['pipe','pipe','pipe']});
const replies=new Map(),events=[];let input='',diagnostics='',counter=0,base,cookie,csrf,epoch,heartbeat,uiBrowser;
helper.stderr.on('data',b=>diagnostics+=b.toString());
helper.stdout.on('data',b=>{input+=b;let i;while((i=input.indexOf('\n'))>=0){const msg=JSON.parse(input.slice(0,i));input=input.slice(i+1);if(msg.event){events.push(msg);continue;}const cb=replies.get(msg.id);assert.ok(cb,'unexpected reply');replies.delete(msg.id);cb(msg);}});
const rpc=q=>new Promise(resolve=>{const id=q.id||`probe-${++counter}`;replies.set(id,resolve);helper.stdin.write(JSON.stringify({...q,id})+'\n');});
const ok=async q=>{const r=await rpc(q);assert.equal(r.ok,true,JSON.stringify(r));return r.result;};
const cid=crypto.randomUUID();
async function local(b,endpoint='/api',options={}){
  const r=await fetch(base+endpoint,{method:'POST',headers:{Origin:base,'Content-Type':'application/json',...(cookie?{Cookie:cookie}:{}),...(csrf?{'X-Helm-CSRF':csrf}:{}),...options.headers},body:JSON.stringify({controller_id:cid,epoch,...b})});
  return r;
}
async function api(b){const r=await local(b);const v=await r.json();assert.equal(r.status,200,JSON.stringify(v));if(v.epoch)epoch=v.epoch;return v;}
const action=(a,id=crypto.randomUUID())=>rpc({id,op:'action',epoch,capture_epoch:epoch,action_sha256:crypto.createHash('sha256').update(JSON.stringify(a)).digest('hex'),action:a});
async function pending(){for(let i=0;i<80;i++){const s=await api({op:'state'});if(s.prompts.length)return s.prompts[0];await new Promise(r=>setTimeout(r,25));}throw Error('No bounded local prompt');}
try{
  const init=await ok({op:'init',session_dir:root,limits:{prompt_timeout_ms:2000,heartbeat_ms:5000}});base=new URL(init.companion_url).origin;epoch=init.epoch;
  heartbeat=setInterval(()=>void rpc({op:'heartbeat'}),700);
  assert.equal(init.mode,'private');
  const publicAsset=await fetch(base+'/');assert.equal(publicAsset.headers.get('cache-control'),'no-store');assert.match(publicAsset.headers.get('content-security-policy'),/frame-ancestors 'none'/);
  assert.equal(await new Promise((resolve,reject)=>http.get(base+'/',{headers:{Host:'evil.invalid'}},res=>{res.resume();resolve(res.statusCode);}).on('error',reject)),403);
  const auth=await local({secret:new URL(init.companion_url).hash.slice(1)},'/session');assert.equal(auth.status,200);cookie=auth.headers.get('set-cookie').split(';')[0];csrf=(await auth.json()).csrf;
  assert.equal((await local({op:'claim'},'/api',{headers:{Origin:'http://evil.invalid'}})).status,403);
  assert.equal((await local({op:'claim'},'/api',{headers:{'X-Helm-CSRF':'invalid'}})).status,403);
  await api({op:'claim'});
  assert.equal((await action({action:'inspect',page_id:null})).ok,false);
  assert.equal((await rpc({op:'control',mode:'agent'})).ok,false);
  await api({op:'origin',url:fixture,private_network:false});
  assert.equal((await local({op:'navigate',url:fixture})).status,403,'private network must fail closed');
  await new Promise(r=>setTimeout(r,250));await api({op:'origin',url:fixture,private_network:true});await api({op:'navigate',url:fixture});
  const frame=await local({},'/frame');assert.equal(frame.status,200);const jpg=Buffer.from(await frame.arrayBuffer());assert.equal(jpg.readUInt16BE(),0xffd8);
  // A second tab/client cannot take over a live local controller.
  assert.equal((await local({op:'claim',controller_id:crypto.randomUUID()})).status,403);
  await api({op:'mode',mode:'agent',confirm_share:true});
  let inspected=await action({action:'inspect',page_id:null});assert.equal(inspected.result.state,'completed');let obs=JSON.parse(inspected.result.text);assert.equal(obs.title,'Local fixture');assert.ok(!obs.refs.some(x=>x.password));
  const target={page_id:obs.page_id,observation_id:obs.observation_id};
  const shot=await action({action:'screenshot',target});assert.equal(shot.result.image.mime_type,'image/jpeg');assert.ok(Buffer.from(shot.result.image.data_base64,'base64').length<=2097152);
  const button=obs.refs.find(x=>x.label==='Count');assert.ok(button);
  const click={action:'click',target,element:button.ref};
  let denied=action(click);await api({op:'confirm',prompt_id:(await pending()).id,allow:false});assert.equal((await denied).result.state,'refused');
  const clickID=crypto.randomUUID(),clicked=action(click,clickID);await api({op:'confirm',prompt_id:(await pending()).id,allow:true});assert.equal((await clicked).result.state,'completed');
  const replay=await action(click,clickID);assert.equal(replay.result.state,'completed');assert.equal(replay.result.text,'duplicate_content_withheld');
  const conflict=await action({action:'inspect',page_id:null},clickID);assert.equal(conflict.error.code,'request_id_conflict');
  assert.equal((await action({action:'screenshot',target})).result.state,'refused');
  inspected=await action({action:'inspect',page_id:obs.page_id});obs=JSON.parse(inspected.result.text);assert.match(obs.text,/Clicked/);
  const fill={action:'fill',target:{page_id:obs.page_id,observation_id:obs.observation_id},element:obs.refs.find(x=>x.label==='Fixture input').ref,text:'remote fixture text'};
  const interrupted=action(fill);await pending();await api({op:'mode',mode:'private'});const cancelled=await interrupted;assert.equal(cancelled.result.state,'cancelled');assert.equal(cancelled.result.image,null);assert.ok(!cancelled.result.text.includes('remote fixture text'));
  assert.equal((await action({action:'inspect',page_id:null})).ok,false);
  // Real local interactive viewport: a separate Chromium renders the companion and forwards keyboard input.
  uiBrowser=await chromium.launch({executablePath:'/usr/bin/chromium',headless:true,chromiumSandbox:true});
  const uiContext=await uiBrowser.newContext();await uiContext.addCookies([{name:'helm_browser',value:cookie.split('=')[1],url:base,httpOnly:true,sameSite:'Strict'}]);
  // Explicit local controller expiry, not automatic forced arbitration.
  await new Promise(r=>setTimeout(r,5300));
  const ui=await uiContext.newPage();await ui.goto(base);await ui.waitForFunction(()=>document.querySelector('#frame').naturalWidth>0);
  await ui.locator('#viewport').click({position:{x:40,y:130}});await ui.keyboard.press('Tab');
  assert.ok(await ui.locator('#frame').evaluate(e=>e.naturalWidth>=640));
  await uiBrowser.close();uiBrowser=null;
  clearInterval(heartbeat);await new Promise(r=>setTimeout(r,5500));assert.equal((await ok({op:'status'})).shared,false);
  const receipts=await fs.readdir(path.join(root,'receipts'));assert.ok(receipts.length>=5);for(const name of receipts){const data=await fs.readFile(path.join(root,'receipts',name),'utf8');assert.ok(!data.includes('remote fixture text'));assert.ok(!data.includes(fixture));assert.ok(!data.includes('private-password-marker'));}
  await ok({op:'shutdown'});helper.stdin.end();await new Promise(r=>helper.once('exit',r));
  assert.equal(diagnostics,'');assert.ok(events.some(e=>e.event==='control'));assert.ok(events.some(e=>e.event==='approval'));
  console.log('PASS: sandboxed persistent Chromium; loopback Host/Origin/CSRF/CSP; explicit private-network grant; real companion rendering/input transport; local controller exclusion; private refusal; inspect/ref/screenshot; local deny/allow; durable duplicate/conflict; takeover privacy fencing; watchdog; clean shutdown.');
  console.log(`Evidence profile and non-content receipts retained privately at ${root}`);
}catch(e){console.error('PROBE FAILED:',e.stack);process.exitCode=1;}
finally{clearInterval(heartbeat);if(uiBrowser)await uiBrowser.close();if(helper.exitCode===null){helper.stdin.end();await new Promise(r=>{helper.once('exit',r);setTimeout(()=>{helper.kill('SIGKILL');r();},5000).unref();});}await new Promise(r=>fixtures.close(r));}
