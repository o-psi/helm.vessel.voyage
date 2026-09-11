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
const websiteUploads=[];let slowStarted=false;
const fixtures=http.createServer((req,res)=> {
  if(req.url==='/uploaded'){let bytes='';req.on('data',b=>bytes+=b);req.on('end',()=>{websiteUploads.push(JSON.parse(bytes));res.end('ok');});return;}
  if(req.url==='/slow'){slowStarted=true;setTimeout(()=>res.end('<title>Slow fixture</title>'),1500);return;}
  if(req.url==='/redirect'){res.writeHead(302,{Location:'http://localhost:1/forbidden'}).end();return;}
  if(req.url==='/empty'){res.writeHead(200,{'Content-Disposition':'attachment; filename="empty.txt"'}).end();return;}
  if(req.url==='/file'){res.writeHead(200,{'Content-Disposition':'attachment; filename="synthetic.txt"','Content-Type':'application/octet-stream'}).end('synthetic local download');return;}
  res.setHeader('Content-Type','text/html');res.end('<!doctype html><title>Local fixture</title><h1>Synthetic page</h1><button id="counter" onclick="this.textContent=\'Clicked\'">Count</button><input id="text" style="position:absolute;left:100px;top:150px" aria-label="Fixture input" oninput="this.nextElementSibling.textContent=this.value"><span id="typed"></span><input type="password" value="private-password-marker"><a href="/file">Download fixture</a><a href="/empty">Empty fixture</a><input id="file" type="file" style="position:absolute;left:100px;top:200px" onchange="this.files[0].text().then(body=>fetch(\'/uploaded\',{method:\'POST\',body:JSON.stringify({name:this.files[0].name,type:this.files[0].type,body})}))">');
});
await new Promise(r=>fixtures.listen(0,'127.0.0.1',r));
const fixture=`http://127.0.0.1:${fixtures.address().port}`;
const helper=spawn(process.execPath,[path.join(here,'helper.mjs')],{stdio:['pipe','pipe','pipe']});
const replies=new Map(),events=[];let transcript='',input='',diagnostics='',counter=0,base,cookie,csrf,epoch,heartbeat,uiBrowser;
helper.stderr.on('data',b=>diagnostics+=b.toString());
helper.stdout.on('data',b=>{transcript+=b;input+=b;let i;while((i=input.indexOf('\n'))>=0){const msg=JSON.parse(input.slice(0,i));input=input.slice(i+1);if(msg.event){events.push(msg);continue;}const cb=replies.get(msg.id);assert.ok(cb,'unexpected reply');replies.delete(msg.id);cb(msg);}});
const rpc=q=>new Promise((resolve,reject)=>{const id=q.id||`probe-${++counter}`;const timer=setTimeout(()=>reject(Error('bounded helper RPC timeout')),25000);replies.set(id,r=>{clearTimeout(timer);resolve(r);});helper.stdin.write(JSON.stringify({...q,id})+'\n');});
const ok=async q=>{const r=await rpc(q);assert.equal(r.ok,true,JSON.stringify(r));return r.result;};
const cid=crypto.randomUUID();
async function local(b,endpoint='/api',options={}){
  const r=await fetch(base+endpoint,{method:'POST',headers:{Origin:base,'Content-Type':'application/json',...(cookie?{Cookie:cookie}:{}),...(csrf?{'X-Helm-CSRF':csrf}:{}),...options.headers},body:JSON.stringify({controller_id:cid,epoch,...b})});
  return r;
}
async function api(b){const r=await local(b);const v=await r.json();assert.equal(r.status,200,JSON.stringify(v));if(v.epoch)epoch=v.epoch;return v;}
const browser_id=crypto.randomUUID(),binding={session_id:crypto.randomUUID(),incarnation:crypto.randomUUID(),browser_id,resource_id:crypto.randomUUID(),executor_id:crypto.randomUUID(),run_id:null,controller_epoch:1,capture_epoch:1,expires_at_ms:Date.now()+600000};let run_id=crypto.randomUUID();
const action=(a,id=crypto.randomUUID())=>rpc({id,op:'action',epoch,capture_epoch:epoch,binding:{...binding,run_id,controller_epoch:epoch,capture_epoch:epoch},action_sha256:crypto.createHash('sha256').update(JSON.stringify(a)).digest('hex'),action:a});
async function pending(){for(let i=0;i<80;i++){const s=await api({op:'state'});if(s.prompts.length)return s.prompts[0];await new Promise(r=>setTimeout(r,25));}throw Error('No bounded local prompt');}

async function restarted(directory,fn){
  const child=spawn(process.execPath,[path.join(here,'helper.mjs')],{stdio:['pipe','pipe','pipe']});let buf='',stderr='';const waiting=new Map();
  child.stderr.on('data',b=>stderr+=b);child.stdout.on('data',b=>{buf+=b;let i;while((i=buf.indexOf('\n'))>=0){const r=JSON.parse(buf.slice(0,i));buf=buf.slice(i+1);if(r.event)continue;waiting.get(r.id)?.(r);waiting.delete(r.id);}});
  const call=q=>new Promise((resolve,reject)=>{const id=crypto.randomUUID(),timer=setTimeout(()=>reject(Error('restart timeout')),20000);waiting.set(id,r=>{clearTimeout(timer);resolve(r);});child.stdin.write(JSON.stringify({...q,id})+'\n');});
  try{await fn(call,await call({op:'init',session_dir:directory}));}finally{if(child.exitCode===null){await call({op:'shutdown'});child.stdin.end();await new Promise(r=>{if(child.exitCode!==null)return r();child.once('exit',r);});}assert.equal(stderr,'');}
}
try{
  const init=await ok({op:'init',session_dir:root,browser_id,binding,limits:{prompt_timeout_ms:2000,heartbeat_ms:5000}});base=new URL(init.companion_url).origin;epoch=init.epoch;
  heartbeat=setInterval(()=>void rpc({op:'heartbeat',binding}),700);
  assert.equal(init.mode,'private');
  const publicAsset=await fetch(base+'/');assert.equal(publicAsset.headers.get('cache-control'),'no-store');assert.match(publicAsset.headers.get('content-security-policy'),/frame-ancestors 'none'/);
  assert.equal(await new Promise((resolve,reject)=>http.get(base+'/',{headers:{Host:'evil.invalid'}},res=>{res.resume();resolve(res.statusCode);}).on('error',reject)),403);
  const auth=await local({secret:new URL(init.companion_url).hash.slice(1)},'/session');assert.equal(auth.status,200);cookie=auth.headers.get('set-cookie').split(';')[0];csrf=(await auth.json()).csrf;
  assert.equal((await local({secret:new URL(init.companion_url).hash.slice(1)},'/session',{headers:{Cookie:''}})).status,403,'one-use session secret');
  assert.equal((await local({op:'claim'},'/api',{headers:{Cookie:'helm_browser=wrong'}})).status,403);
  assert.equal((await local({op:'claim'},'/api',{headers:{Origin:'http://evil.invalid'}})).status,403);
  assert.equal((await local({op:'claim'},'/api',{headers:{'X-Helm-CSRF':'invalid'}})).status,403);
  await api({op:'claim'});
  assert.equal((await action({action:'inspect',page_id:null})).ok,false);
  assert.equal((await rpc({op:'control',mode:'agent'})).ok,false);
  await api({op:'origin',url:fixture,private_network:false});
  assert.equal((await local({op:'navigate',url:fixture})).status,403,'private network must fail closed');
  await new Promise(r=>setTimeout(r,250));await api({op:'origin',url:fixture,private_network:true});await api({op:'navigate',url:fixture});
  assert.equal((await local({op:'navigate',url:fixture+'/redirect'})).status,403,'redirect origin cannot inherit grant');await new Promise(r=>setTimeout(r,250));await api({op:'navigate',url:fixture});
  const frame=await local({},'/frame');assert.equal(frame.status,200,frame.status!==200?await frame.text():'');const jpg=Buffer.from(await frame.arrayBuffer());assert.equal(jpg.readUInt16BE(),0xffd8);
  // A second tab/client cannot take over a live local controller.
  assert.equal((await local({op:'claim',controller_id:crypto.randomUUID()})).status,403);
  // A local picker file reaches the website through the very same visual/input session.
  await api({op:'pointer',kind:'down',x:150,y:210,button:0});await api({op:'pointer',kind:'up',x:150,y:210,button:0});
  let chooser;for(let i=0;i<40;i++){chooser=(await api({op:'state'})).chooser;if(chooser)break;await new Promise(r=>setTimeout(r,25));}assert.ok(chooser);
  await api({op:'upload',chooser_id:chooser,name:'local.txt',data_base64:Buffer.from('synthetic human upload').toString('base64')});
  for(let i=0;i<40&&!websiteUploads.length;i++)await new Promise(r=>setTimeout(r,25));assert.deepEqual(websiteUploads,[{name:'local.txt',type:'application/octet-stream',body:'synthetic human upload'}]);
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
  // Remote bytes require one local prepare approval, a separate grant and an upload effect approval.
  const prepare=action({action:'upload_prepare',transfer_id:crypto.randomUUID(),name:'fixture.txt',mime_type:'text/plain',data_base64:Buffer.from('synthetic remote upload').toString('base64')});
  const preparePrompt=await pending();assert.equal(preparePrompt.summary.filename,'fixture.txt');assert.equal(preparePrompt.summary.mime_type,'text/plain');assert.equal(preparePrompt.summary.bytes,Buffer.byteLength('synthetic remote upload'));assert.equal(preparePrompt.summary.current_origin,fixture);await api({op:'confirm',prompt_id:preparePrompt.id,allow:true});const prepared=await prepare;assert.equal(prepared.result.state,'completed');const grant=JSON.parse(prepared.result.text).grant_id;
  await api({op:'share_upload',upload_id:grant,allow:true});
  obs=JSON.parse((await action({action:'inspect',page_id:obs.page_id})).result.text);assert.ok(obs.uploads.some(u=>u.grant_id===grant));
  const upload=action({action:'upload',target:{page_id:obs.page_id,observation_id:obs.observation_id},element:obs.refs.find(r=>r.type==='file').ref,grant_id:grant});const uploadPrompt=await pending();assert.equal(uploadPrompt.summary.filename,'fixture.txt');assert.equal(uploadPrompt.summary.destination_origin,fixture);assert.equal(uploadPrompt.summary.element.type,'file');await api({op:'confirm',prompt_id:uploadPrompt.id,allow:true});assert.equal((await upload).result.state,'completed');
  for(let i=0;i<40&&websiteUploads.length<2;i++)await new Promise(r=>setTimeout(r,25));assert.deepEqual(websiteUploads,[{name:'local.txt',type:'application/octet-stream',body:'synthetic human upload'},{name:'fixture.txt',type:'text/plain',body:'synthetic remote upload'}]);
  obs=JSON.parse((await action({action:'inspect',page_id:obs.page_id})).result.text);
  const downloadClick=action({action:'click',target:{page_id:obs.page_id,observation_id:obs.observation_id},element:obs.refs.find(r=>r.label==='Download fixture').ref});await api({op:'confirm',prompt_id:(await pending()).id,allow:true});assert.equal((await downloadClick).result.state,'completed');
  let localState;for(let i=0;i<80;i++){localState=await api({op:'state'});if(localState.downloads.some(d=>!d.pending))break;await new Promise(r=>setTimeout(r,25));}
  const downloaded=localState.downloads.find(d=>!d.pending);assert.ok(downloaded,'download staged');
  const saved=await local({download_id:downloaded.id},'/save');assert.equal(await saved.text(),'synthetic local download');
  obs=JSON.parse((await action({action:'inspect',page_id:obs.page_id})).result.text);assert.equal(obs.downloads.length,0,'local save does not disclose');
  await api({op:'disclose_download',download_id:downloaded.id,allow:true});
  const disclose=action({action:'download',target:{page_id:obs.page_id,observation_id:obs.observation_id},download_id:downloaded.id});const disclosePrompt=await pending();assert.equal(disclosePrompt.summary.filename,'synthetic.txt');assert.equal(disclosePrompt.summary.bytes,Buffer.byteLength('synthetic local download'));await api({op:'confirm',prompt_id:disclosePrompt.id,allow:true});const disclosed=await disclose;assert.equal(disclosed.result.state,'completed');assert.equal(disclosed.result.file.name,'synthetic.txt');assert.equal(Buffer.from(disclosed.result.file.data_base64,'base64').toString(),'synthetic local download');
  obs=JSON.parse((await action({action:'inspect',page_id:obs.page_id})).result.text);
  const emptyClick=action({action:'click',target:{page_id:obs.page_id,observation_id:obs.observation_id},element:obs.refs.find(r=>r.label==='Empty fixture').ref});await api({op:'confirm',prompt_id:(await pending()).id,allow:true});assert.equal((await emptyClick).result.state,'completed');
  let empty;for(let i=0;i<80;i++){empty=(await api({op:'state'})).downloads.find(d=>d.name==='empty.txt'&&!d.pending);if(empty)break;await new Promise(r=>setTimeout(r,25));}assert.ok(empty);assert.equal(await (await local({download_id:empty.id},'/save')).text(),'');
  await api({op:'disclose_download',download_id:empty.id,allow:true});obs=JSON.parse((await action({action:'inspect',page_id:obs.page_id})).result.text);
  const emptyDisclosure=action({action:'download',target:{page_id:obs.page_id,observation_id:obs.observation_id},download_id:empty.id});await api({op:'confirm',prompt_id:(await pending()).id,allow:true});const emptyResult=await emptyDisclosure;assert.equal(emptyResult.result.state,'refused');assert.equal(emptyResult.result.text,'empty_download_local_save_only');assert.equal(emptyResult.result.file,null);
  const fill={action:'fill',target:{page_id:obs.page_id,observation_id:obs.observation_id},element:obs.refs.find(x=>x.label==='Fixture input').ref,text:'remote fixture text'};
  const promptStart=Date.now(),unattended=action(fill);await pending();assert.equal((await unattended).result.state,'refused');assert.ok(Date.now()-promptStart<4000,'local prompt deadline is bounded');
  const interrupted=action(fill);const fillPrompt=await pending();assert.equal(fillPrompt.summary.text,'remote fixture text');assert.equal(fillPrompt.summary.element.label,'Fixture input');assert.equal(fillPrompt.summary.current_origin,fixture);await api({op:'mode',mode:'private'});const cancelled=await interrupted;assert.equal(cancelled.result.state,'cancelled');assert.equal(cancelled.result.image,null);assert.ok(!cancelled.result.text.includes('remote fixture text'));
  assert.equal((await action({action:'inspect',page_id:null})).ok,false);
  // Idle heartbeats never erase the first actual run pin; a later run must explicitly reshare.
  await api({op:'mode',mode:'agent',confirm_share:true});await action({action:'inspect',page_id:null});await ok({op:'heartbeat',binding});
  run_id=crypto.randomUUID();assert.equal((await action({action:'inspect',page_id:null})).error.code,'run_changed');assert.equal((await ok({op:'status'})).shared,false);
  await api({op:'state'});await api({op:'mode',mode:'agent',confirm_share:true});obs=JSON.parse((await action({action:'inspect',page_id:null})).result.text);
  const wrong={id:crypto.randomUUID(),op:'action',epoch,capture_epoch:epoch,binding:{...binding,run_id,session_id:crypto.randomUUID(),controller_epoch:epoch,capture_epoch:epoch},action_sha256:'a'.repeat(64),action:{action:'inspect',page_id:null}};
  assert.equal((await rpc(wrong)).error.code,'binding_fenced');
  assert.equal((await rpc({...wrong,id:crypto.randomUUID(),binding:{...wrong.binding,session_id:binding.session_id,expires_at_ms:null}})).error.code,'invalid_binding');
  // A fresh actual raster races control; no capture survives the epoch fence.
  const capture=action({action:'screenshot',target:{page_id:obs.page_id,observation_id:obs.observation_id}});await ok({op:'control',mode:'private'});const fencedCapture=await capture;assert.equal(fencedCapture.result.image,null);assert.notEqual(fencedCapture.result.state,'completed');
  await api({op:'state'});await api({op:'mode',mode:'agent',confirm_share:true});obs=JSON.parse((await action({action:'inspect',page_id:null})).result.text);
  // Take over AFTER the durable dispatched journal, while navigation is actually in flight.
  const slowID=crypto.randomUUID(),slow=action({action:'navigate',target:{page_id:obs.page_id,observation_id:obs.observation_id},url:fixture+'/slow'},slowID);
  await api({op:'confirm',prompt_id:(await pending()).id,allow:true});
  for(let i=0;i<80&&!slowStarted;i++)await new Promise(r=>setTimeout(r,10));assert.ok(slowStarted);
  assert.equal((await ok({op:'receipt',request_id:slowID})).receipt.state,'dispatched');
  await api({op:'mode',mode:'private'});assert.equal((await local({op:'text',text:'must not race automation'})).status,403);
  const slowResult=await slow;assert.equal(slowResult.result.state,'unresolved');assert.equal(slowResult.result.image,null);assert.ok(!slowResult.result.text.includes('Slow fixture'));
  await api({op:'navigate',url:fixture});
  const disposable=await api({op:'upload',name:'discard.txt',data_base64:'eA=='});await api({op:'discard_upload',transfer_id:disposable.upload_id});await api({op:'discard_download',transfer_id:downloaded.id});await api({op:'discard_download',transfer_id:empty.id});assert.equal((await api({op:'state'})).uploads.length,0);assert.equal((await api({op:'state'})).downloads.length,0);
  // Real local interactive viewport: a separate Chromium renders the companion and forwards keyboard input.
  uiBrowser=await chromium.launch({executablePath:'/usr/bin/chromium',headless:true,chromiumSandbox:true});
  const uiContext=await uiBrowser.newContext();await uiContext.addCookies([{name:'helm_browser',value:cookie.split('=')[1],url:base,httpOnly:true,sameSite:'Strict'}]);
  // Explicit local controller expiry, not automatic forced arbitration.
  await new Promise(r=>setTimeout(r,5300));
  const ui=await uiContext.newPage();await ui.goto(base);await ui.waitForFunction(()=>document.querySelector('#frame').naturalWidth>0);
  const box=await ui.locator('#viewport').boundingBox();await ui.locator('#viewport').click({position:{x:150*box.width/1280,y:160*box.height/720}});await ui.keyboard.type('LOCAL-TYPED');await new Promise(r=>setTimeout(r,1000));assert.equal(await ui.locator('#error').textContent(),'');assert.ok(!transcript.includes('LOCAL-TYPED'),'private input never enters helper stdout');
  await new Promise(r=>setTimeout(r,500));ui.on('dialog',d=>d.accept());await ui.locator('#share').click();await ui.waitForFunction(()=>document.querySelector('#mode').textContent.startsWith('AGENT'));epoch=(await ok({op:'status'})).epoch;const afterTyping=await action({action:'inspect',page_id:obs.page_id});assert.match(JSON.parse(afterTyping.result.text).text,/LOCAL-TYPED/);
  assert.ok(await ui.locator('#frame').evaluate(e=>e.naturalWidth>=640));
  await uiBrowser.close();uiBrowser=null;
  clearInterval(heartbeat);await new Promise(r=>setTimeout(r,5500));assert.equal((await ok({op:'status'})).shared,false);await ok({op:'heartbeat',binding});assert.equal((await ok({op:'status'})).shared,false,'heartbeat cannot restore sharing');assert.equal((await action({action:'inspect',page_id:null})).ok,false);
  const receipts=await fs.readdir(path.join(root,'receipts'));assert.ok(receipts.length>=5);for(const name of receipts){assert.equal((await fs.stat(path.join(root,'receipts',name))).mode&0o077,0);const data=await fs.readFile(path.join(root,'receipts',name),'utf8');assert.ok(!data.includes('remote fixture text'));assert.ok(!data.includes(fixture));assert.ok(!data.includes('private-password-marker'));}
  assert.equal((await fs.stat(root)).mode&0o077,0);assert.equal((await fs.stat(path.join(root,'executor.lock'))).mode&0o077,0);
  await ok({op:'shutdown'});helper.stdin.end();await new Promise(r=>helper.once('exit',r));
  assert.equal(diagnostics,'');assert.ok(events.some(e=>e.event==='control'));assert.ok(events.some(e=>e.event==='approval'));assert.ok(events.filter(e=>e.event==='approval').every(e=>Object.keys(e).sort().join(',')==='event,pending,request_id'),'approval content stays local');
  // A clean restart reuses receipts without replay; uncertain historical effects remain uncertain.
  const synthetic=crypto.randomUUID(),queued=crypto.randomUUID();for(const [id,state]of [[synthetic,'dispatched'],[queued,'queued']])await fs.writeFile(path.join(root,'receipts',id+'.json'),JSON.stringify({id,digest:'a'.repeat(64),action_sha256:'b'.repeat(64),epoch:1,created_at:Date.now(),state}),{mode:0o600});
  await restarted(root,async(call,r)=>{assert.equal(r.ok,true);assert.equal((await call({op:'receipt',request_id:synthetic})).result.receipt.state,'unknown');assert.equal((await call({op:'receipt',request_id:queued})).result.receipt.state,'cancelled_before_dispatch');assert.equal((await call({op:'receipt',request_id:clickID})).result.receipt.state,'completed');});
  // This lock is an explicitly created probe fixture, NOT an orphan selected for guessed cleanup.
  const lockedRoot=await fs.mkdtemp(path.join(path.dirname(root),'helm-browser-locked-'));await fs.chmod(lockedRoot,0o700);await fs.writeFile(path.join(lockedRoot,'executor.lock'),'synthetic unexpected lock',{mode:0o600});
  await restarted(lockedRoot,async(_call,r)=>assert.equal(r.error.code,'profile_locked'));assert.equal(await fs.readFile(path.join(lockedRoot,'executor.lock'),'utf8'),'synthetic unexpected lock');
  const budgetRoot=await fs.mkdtemp(path.join(path.dirname(root),'helm-browser-budget-'));await fs.chmod(budgetRoot,0o700);
  await restarted(budgetRoot,async(call,r)=>{assert.equal(r.ok,true);const f=await fs.open(path.join(budgetRoot,'synthetic-budget-file'),'wx',0o600);await f.truncate(257*1024*1024);await f.close();let fenced;for(let i=0;i<50;i++){await new Promise(r=>setTimeout(r,100));fenced=await call({op:'status'});if(!fenced.ok)break;}assert.equal(fenced.error?.code,'not_initialized','live session disk accounting closes Chromium');});
  const checkPrivate=async dir=>{for(const entry of await fs.readdir(dir,{withFileTypes:true})){const file=path.join(dir,entry.name),stat=await fs.lstat(file);if(stat.isSymbolicLink())continue;assert.equal(stat.mode&0o077,0,'session files remain private');if(stat.isDirectory())await checkPrivate(file);assert.ok(!/trace|\.webm$/.test(entry.name),'no recording or trace artifact');}};await checkPrivate(root);
  console.log('PASS: sandboxed persistent Chromium; loopback Host/Origin/CSRF/CSP; explicit private-network grant; real companion rendering/input transport; local controller exclusion; private refusal; inspect/ref/screenshot; local deny/allow; durable duplicate/conflict; staged upload/grant and local-save versus remote-disclosure; verified keyboard input and private stdout withholding; screenshot/control race; in-flight navigation takeover; idle/active/later run binding and explicit reshare; prompt and heartbeat deadlines; clean restart and unknown/no-replay receipts; untouched unexpected lock; private filesystem; live disk budget shutdown.');
  console.log(`Evidence profile and non-content receipts retained privately at ${root}`);
}catch(e){console.error('PROBE FAILED:',e.stack);process.exitCode=1;}
finally{clearInterval(heartbeat);if(uiBrowser)await uiBrowser.close();if(helper.exitCode===null){helper.stdin.end();await new Promise(r=>{helper.once('exit',r);setTimeout(()=>{helper.kill('SIGKILL');r();},5000).unref();});}await new Promise(r=>fixtures.close(r));}
