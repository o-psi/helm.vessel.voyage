// Production shared viewer and native Helm launcher over real authenticated Vessel sockets.
import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
import {randomUUID,createHash} from 'node:crypto';
import assert from 'node:assert/strict';
const require=createRequire(import.meta.url);
const cfg=JSON.parse(fs.readFileSync(process.argv[2]));
const WebSocket=require(cfg.ws);
const {chromium}=require(cfg.playwright);
const peers=[], evidence={steps:[],pre_teardown:[],cleanup:{}};
const webDiagnostics=[];
let browser;
const save=()=>fs.writeFileSync(cfg.evidence,JSON.stringify(evidence,null,2));
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const failed=response=>response.error!=null||response.result?.error!=null;
const sha256=bytes=>createHash('sha256').update(bytes).digest('hex');
function bindingEpochs(binding){
 return Object.fromEntries(['document_epoch','viewport_epoch','controller_epoch','capture_epoch']
  .filter(key=>Number.isSafeInteger(binding?.[key])).map(key=>[key,binding[key]]));
}
function publicRefusalCategory(value){
 if(value==null)return null;
 for(const code of ['stale_reference','element_hidden','element_disabled','element_not_editable','element_obscured','observation_unavailable']){
  if(typeof value==='string'&&value.startsWith('Browser action refused before interaction ('+code+')'))return code;
 }
 const known=['stale browser binding','browser input sequence mismatch','browser transport unresolved',
  'browser operation refused or outcome unknown','browser mirror authority changed',
  'browser attachment missing','browser attachment authority mismatch','browser viewer admission unavailable',
  'browser socket disconnected','browser pending capacity or identity conflict','browser capacity busy',
  'prior host browser worker lock remains; inspect cleanup before recovery',
  'browser queue full; request not dispatched','browser outcome unknown; never replay'];
 return known.includes(value)?value:'other refusal';
}
function valueOutcome(value){
 if(value==null)return {kind:'null'};
 if(value.encoding==='gzip')return {kind:'mirror_batch',reset:value.reset===true,
  cursor:Number.isSafeInteger(value.cursor)?value.cursor:null,
  latest:Number.isSafeInteger(value.latest)?value.latest:null};
 return {kind:typeof value==='object'?'object':'other'};
}
function retainWebDiagnostics(){
 evidence.webOperationTrace=webDiagnostics.map(trace=>trace.filter((entry,index)=>
  index>=trace.length-32||!['status','mirror','snapshot'].includes(entry.action)).map(entry=>({
  action:entry.action,input_kind:entry.input_kind,mode:entry.mode,
  refused:entry.refused,outcome_unknown:entry.outcome_unknown,transport_failed:entry.transport_failed,
  requested_epochs:entry.requested_epochs,reply_epochs:entry.reply_epochs,
  outer_refusal_category:entry.outer_refusal_category,inner_refusal_category:entry.inner_refusal_category,
  value_outcome:entry.value_outcome,
  ...(entry.client_state?{client_state:entry.client_state}:{}),
 })));
}
async function fixtureControl(action,item,extra={}){
 const response=await fetch(cfg.site+'/fixture-control',{method:'POST',
  headers:{'Content-Type':'application/json',Authorization:'Bearer '+cfg.fixture_control_token},
  body:JSON.stringify({action,session:item.session,...extra}),signal:AbortSignal.timeout(15000)});
 assert.equal(response.status,200,'fixture control refused; private evidence retained');
 const result=await response.json();assert.equal(result.error,undefined);return result;
}
async function nativeMetadata(page){
 await page.addInitScript(()=>{
  const original=window.fetch;window.fixtureBrowser=null;window.fixtureContext=null;window.fixtureOperations=[];window.fixtureEffectsPending=0;
  window.fetch=async(...args)=>{
   const operation=args[0]==='/operation'?JSON.parse(args[1].body):null;
   const effect=operation&&!['status','mirror','receipt'].includes(operation.action);
   if(effect)window.fixtureEffectsPending++;
   try{
    const response=await original(...args);
    if(operation){
     const reply=response.ok?await response.clone().json():null,status=reply?.result?.status;
     if(reply?.context)window.fixtureContext=reply.context;
     // Tab identity/title and page URL are ephemeral synthetic fixture state,
     // used for confirmed UI sequencing, never appended to operation traces.
     if(status)window.fixtureBrowser={binding:status.binding,mode:status.mode,
      controls:!!status.binding&&status.controller===status.binding.attachment_id,
      running:status.running,input_sequence:status.input_sequence,tabs:status.tabs,tab_details:status.tab_details,page:status.page,
      downloads:status.downloads?.map(d=>d.id)||[]};
     if(window.fixtureOperations.length>=256)window.fixtureOperations.shift();
     window.fixtureOperations.push({action:operation.action,input_kind:operation.input?.type,
      command_id:operation.command_id,http:response.status});
    }return response;
   }finally{if(effect)window.fixtureEffectsPending--;}
  };
 });
}
async function readyInput(page,{claim=false}={}){
 await page.waitForFunction(claim=>{
  const s=window.mounted?.session;
  return s?(s.canInput||claim&&s.canClaim&&s.streaming)&&!s.sending&&s.queue.length===0:
   window.fixtureEffectsPending===0&&document.querySelector('.browser-next-frames')?.dataset.control==='true';
 },claim);
}
async function tabState(page){
 return page.evaluate(()=>{
  const status=window.mounted?.session.status||window.fixtureBrowser;
  return {id:status?.binding?.tab_id,tabs:status?.tabs||[],details:status?.tab_details||[],page:status?.page};
 });
}
// This adapter changes only delivery timing of already-confirmed read replies.
// It never delays/mutates an input, manufactures status or retries an effect.
function mirrorGate(){
 const pending=new Set();
 return {holding:false,entered:0,peak:0,
  async read(response){
   if(!this.holding)return response;
   this.entered++;let timer,release;
   const waiting=new Promise(resolve=>{release=()=>{clearTimeout(timer);pending.delete(release);resolve();};timer=setTimeout(release,32000);});
   pending.add(release);this.peak=Math.max(this.peak,pending.size);
   await waiting;return response;
  },release(){this.holding=false;for(const release of [...pending])release();},
 };
}
async function mountedWeb(item,{gate=null,ready=true,refresh=false}={}){
 const peer=await connect({...item},false),page=await browser.newPage(),trace=[];
 webDiagnostics.push(trace);
 await page.exposeFunction('webExchange',async request=>{
  const operation=request.command.operation;
  if(trace.length>=512)throw Error('fixture command bound exceeded');
  const entry={action:operation?.action||request.command.op,
   input_kind:operation?.input?.type,command_id:operation?.command_id,requested_epochs:bindingEpochs(operation?.binding)};trace.push(entry);
  let response;
  try{response=await peer.exchange(request);}catch(error){entry.transport_failed=true;throw error;}
  entry.refused=failed(response);entry.outcome_unknown=response.outcome_unknown===true||response.result?.outcome_unknown===true;
  entry.outer_refusal_category=publicRefusalCategory(response.error);
  entry.inner_refusal_category=publicRefusalCategory(response.result?.error);
  entry.reply_epochs=bindingEpochs(response.result?.result?.status?.binding);
  entry.value_outcome=valueOutcome(response.result?.result?.value);
  if(response.result?.incarnation)peer.item.incarnation=response.result.incarnation;
  if(response.result?.result?.status?.binding)peer.binding=response.result.result.status.binding;
  return gate&&operation?.action==='mirror'?gate.read(response):response;
 });
 await page.goto(cfg.site+'/receiver');
 await page.evaluate(async({item,refresh})=>{
  const {mountHostBrowser}=await import('/web/resources/js/host-browser.js');
  window.mounted=mountHostBrowser(document.querySelector('main'),{client:{exchange:window.webExchange},
   sessionId:item.session,incarnation:item.incarnation,context:()=>({revision:item.revision??0,refresh})});
 },{item:peer.item,refresh});
 if(ready){
  await page.waitForFunction(()=>!window.mounted.session.busy&&(window.mounted.session.attached||window.mounted.session.issue));
  const attached=await page.evaluate(()=>window.mounted.session.attached),issue=await page.evaluate(()=>window.mounted.session.issue);
  assert.equal(attached,true,'fixture Web mount failed: '+(['connect-unknown','status-unavailable','page-unavailable'].includes(issue)?issue:'unavailable'));
  await decoded(page,'Web adapter');
 }
 return {peer,page,trace};
}
async function viewerCapacity(item){
 const fillers=[];let denied;
 try{
  // Call only while native + the observer + Web are attached to this browser.
  fillers.push(await connect({...item}));
  denied=await mountedWeb(item,{ready:false});
  await denied.page.waitForFunction(()=>window.mounted.session.issue==='connect-unknown'&&!window.mounted.session.busy);
  assert.equal(await denied.page.evaluate(()=>window.mounted.session.canInput),false);
  assert.equal(await denied.page.locator('.browser-next-recovery').isVisible(),true);
  assert.equal(denied.trace.filter(t=>t.action==='attach').length,1,'refused admission must not retry itself');
  assert.equal(denied.trace.some(t=>t.action==='start'||t.action==='input'),false,'viewer capacity must not restart or act on the browser');
  assert.equal(denied.trace.find(t=>t.action==='attach').refused,true);
  evidence.steps.push({viewer_capacity:4,fifth_viewer_refused:true,client_recovery_visible:true,no_restart_or_input:true});save();
 }finally{
  if(denied){await denied.page.evaluate(()=>window.mounted.dispose());await denied.page.close();}
  for(const p of fillers){clearInterval(p.leaseTimer);await p.op('detach');}
 }
}
async function clientBreadth(page,label,{observer=null,other=null,peer=null}={}){
 await navigate(page,cfg.site+'/ui/'+label);
 const frame=page.frameLocator('.browser-next-mirror iframe');
 await frame.locator('#count').waitFor();
 await readyInput(page);
 const old=peer?await page.evaluate(()=>{
  const s=window.mounted.session;
  return {binding:{...s.status.binding},node:s.replayer.getMirror().getId(s.replayer.iframe.contentDocument.querySelector('#counter'))};
 }):null;
 const bytes=Buffer.from('SYNTHETIC_BROWSER_FILE_333:'+label+'\n'),name='fixture-'+label+'.txt';
 const chooser=page.waitForEvent('filechooser',{timeout:12000});
 await frame.getByLabel('Upload fixture file',{exact:true}).click();
 await (await chooser).setFiles({name,mimeType:'text/plain',buffer:bytes});
 await frame.locator('#upload-result').filter({hasText:name+'|'+bytes.toString()}).waitFor();
 assert.equal(await frame.locator('#upload-result').textContent(),name+'|'+bytes.toString(),'exact task-side upload bytes');
 evidence.clientStep={client:label,phase:'upload_confirmed'};save();
 // A replayed DOM mutation can precede completion of its input ACK or a
 // concurrent viewport fence. Observe readiness before the next one-shot input.
 await readyInput(page);
 await frame.getByRole('link',{name:'Download fixture bytes',exact:true}).click();
 await readyInput(page);
 await page.waitForFunction(expected=>{
  const status=window.mounted?.session.status||window.fixtureBrowser;
  return status?.downloads?.length===1&&(!window.mounted||status.downloads[0].name===expected);
 },name);
 evidence.clientStep={client:label,phase:'download_capability_confirmed'};save();
 const menu=page.locator('details');
 if(await menu.getAttribute('open')===null)await page.getByLabel('More browser options',{exact:true}).click();
 const delivery=page.getByRole('button',{name:'Download '+name,exact:true});await delivery.waitFor();
 const downloadId=peer?await page.evaluate(()=>window.mounted.session.status.downloads[0].id):null;
 if(observer)assert.deepEqual((await observer.op('status')).status.downloads,[],'another attachment must not receive controller downloads');
 if(other)assert.deepEqual((await other.op('status')).status.downloads,[],'another Voyage must not receive downloads');
 await readyInput(page);
 const downloaded=page.waitForEvent('download',{timeout:12000});await delivery.click();
 const download=await downloaded;assert.equal(download.suggestedFilename(),name);
 const destination=path.join(path.dirname(cfg.evidence),'client-'+label+'.bin');
 await download.saveAs(destination);assert.deepEqual(fs.readFileSync(destination),bytes);
 await page.waitForFunction(()=>document.querySelectorAll('.browser-next-downloads button').length===0);
 // The last delivered capability is absent, rather than a hidden second copy.
 assert.equal(await delivery.count(),0,'consumed download must not remain deliverable');
 if(peer){
  const current=await page.evaluate(()=>({binding:{...window.mounted.session.status.binding},sequence:window.mounted.session.sequence}));
  const retry=await peer.exchange({protocol:1,command:{op:'host_browser',session_id:peer.item.session,incarnation:peer.item.incarnation,
   operation:{action:'input',command_id:randomUUID(),binding:current.binding,sequence:current.sequence+1,claim:false,
    input:{type:'download',download_id:downloadId}}}});
  assert.equal(failed(retry),true,'consumed capability must refuse another delivery');
  assert.equal(retry.result?.result?.value?.data_base64,undefined);
  // This negative probe is a new explicit capability check. Its refused input
  // sequence is observed before the next UI action; no effect is replayed.
  await page.evaluate(()=>window.mounted.session.refresh());
 }
 await readyInput(page);
 await frame.locator('#scroll-anchor').hover();await page.mouse.wheel(0,600);
 await frame.locator('#scroll-result').filter({hasText:/Scrolled:[1-9][0-9]*/}).waitFor();
 await readyInput(page);
 await frame.locator('#scroll-result').hover();await page.mouse.wheel(0,-1200);
 await frame.locator('#scroll-result').filter({hasText:'Scrolled:0'}).waitFor();
 await readyInput(page);
 const original=await tabState(page),originalTitle=original.details.find(t=>t.id===original.id)?.title;
 assert.ok(original.id&&originalTitle,'original task tab must have a confirmed identity/title');
 await page.getByRole('button',{name:'New tab',exact:true}).click();
 await page.waitForFunction(old=>{
  const s=window.mounted?.session.status||window.fixtureBrowser;
  return s?.tabs?.length===2&&s.binding.tab_id!==old;
 },original.id);
 // The new tab can appear in chrome before its full mirror is input-ready.
 // Sending Go then may be refused locally; do not resend that mutation.
 await readyInput(page);
 const created=await tabState(page),createdId=created.id;
 evidence.tabStep={phase:'new_tab_confirmed',selected_new_identity:true};save();
 await navigate(page,cfg.site+'/history-one');
 await page.waitForFunction(expected=>{
  const s=window.mounted?.session.status||window.fixtureBrowser;
  const detail=s?.tab_details?.find(t=>t.id===expected.id);
  const title=document.querySelector('.browser-next-mirror iframe')?.contentDocument?.title;
  return s?.binding?.tab_id===expected.id&&s.page?.url===expected.url&&!!title&&detail?.title===title&&s.page.title===title;
 },{id:createdId,url:cfg.site+'/history-one'});
 await frame.locator('p').filter({hasText:'/history-one'}).waitFor();
 await readyInput(page);
 const loaded=await tabState(page),createdTitle=loaded.details.find(t=>t.id===createdId)?.title;
 assert.ok(createdTitle,'loaded new tab must have an authoritative title');
 evidence.tabStep={phase:'new_tab_navigation_confirmed',selected_new_identity:true,task_document_loaded:true};save();
 await page.getByRole('group',{name:'Browser tabs',exact:true}).getByRole('button',{name:originalTitle,exact:true}).click();
 await page.waitForFunction(id=>(window.mounted?.session.status||window.fixtureBrowser)?.binding?.tab_id===id,original.id);
 await frame.locator('#count').waitFor();
 await readyInput(page);
 await page.getByRole('button',{name:'Close '+createdTitle,exact:true}).click();
 await page.waitForFunction(expected=>{
  const s=window.mounted?.session.status||window.fixtureBrowser;
  return s?.tabs?.length===1&&s.binding.tab_id===expected.original&&!s.tabs.includes(expected.closed);
 },{original:original.id,closed:createdId});
 evidence.tabStep={phase:'created_tab_close_confirmed',original_selected:true,created_identity_retired:true};save();
 assert.equal(await page.locator('.browser-next-tab-close').isDisabled(),true,'last tab remains protected');
 if(old){
  const before=await page.evaluate(()=>window.mounted.session.sequence);
  const response=await peer.exchange({protocol:1,command:{op:'host_browser',session_id:peer.item.session,incarnation:peer.item.incarnation,
   operation:{action:'input',command_id:randomUUID(),binding:old.binding,sequence:before+1,claim:false,
    input:{type:'click',node_id:old.node,button:'left'}}}});
  assert.equal(failed(response),true,'old document/tab fence must refuse');
  assert.equal(await frame.locator('#count').textContent(),'0','stale click must have no task effect');
  const viewport=await page.evaluate(()=>{
   const s=window.mounted.session;return {binding:{...s.status.binding},node:s.replayer.getMirror().getId(s.replayer.iframe.contentDocument.querySelector('#counter'))};
  });
  await page.setViewportSize({width:640,height:720});
  await page.waitForFunction(epoch=>window.mounted.session.status.binding.viewport_epoch>epoch&&window.mounted.session.canInput,viewport.binding.viewport_epoch);
  const resized=await page.evaluate(()=>window.mounted.session.sequence);
  const rejected=await peer.exchange({protocol:1,command:{op:'host_browser',session_id:peer.item.session,incarnation:peer.item.incarnation,
   operation:{action:'input',command_id:randomUUID(),binding:viewport.binding,sequence:resized+1,claim:false,
    input:{type:'click',node_id:viewport.node,button:'left'}}}});
  assert.equal(failed(rejected),true,'old viewport fence must refuse');
  assert.equal(await frame.locator('#count').textContent(),'0','old viewport click must have no task effect');
  await page.setViewportSize({width:1280,height:800});
  await page.waitForFunction(()=>window.mounted.session.canInput&&window.mounted.session.status.viewport.width>640);
 }
 await readyInput(page);
 await frame.getByRole('button',{name:'Increment task counter',exact:true}).click();
 await frame.locator('#count').filter({hasText:'1'}).waitFor();
 assert.equal(await frame.locator('#count').textContent(),'1','fresh element reaches the selected task tab once');
 evidence.steps.push({client_ui:label,upload_exact_bytes:true,download_exact_bytes:true,
  file_sha256:sha256(bytes),download_one_use_ui:true,download_other_attachment_excluded:!!observer,
  download_other_voyage_excluded:!!other,task_scroll:true,tabs_new_select_close:true,last_tab_protected:true,
  consumed_download_refused:!!peer,stale_tab_document_fence_refused:!!old,stale_viewport_fence_refused:!!old,
  fresh_element_effect_once:true});save();
}
async function connect(item, attach=true){
 const cred=JSON.parse(fs.readFileSync(item.access));
 const ws=new WebSocket(cred.endpoint.replace(/^http/,'ws')+'/v1/vessel/socket','voyage.vessel.v1',{headers:{Authorization:`Bearer ${cred.token}`,...(cred.grant_id?{'x-voyage-grant':cred.grant_id}:{})}});
 const pending=new Map();
 const ready=new Promise((resolve,reject)=>{ws.on('error',reject);ws.on('message',raw=>{const m=JSON.parse(raw);if(m.type==='hello')resolve(m);if(m.type==='reply'){const p=pending.get(m.request_id);if(p){pending.delete(m.request_id);clearTimeout(p.timer);p.resolve(m.response);}}});});
 await Promise.race([ready,new Promise((_,r)=>setTimeout(()=>r(Error('socket hello timeout')),12000))]);
 const p={ws,item,binding:null};peers.push(p);
 p.exchange=async request=>{const request_id=randomUUID();return await new Promise((resolve,reject)=>{const timer=setTimeout(()=>{pending.delete(request_id);reject(Error('socket reply timeout (effect not replayed)'));},40000);pending.set(request_id,{resolve,timer});ws.send(JSON.stringify({type:'command',request_id,request}));});};
 p.command=async command=>{const response=await p.exchange({protocol:1,command:{...command,session_id:item.session,incarnation:item.incarnation}});assert.equal(response.error,null,JSON.stringify(response));const reply=response.result;assert.ok(reply.error==null,JSON.stringify(reply));return reply.result;};
 p.op=async(action,extra={})=>{const operation={action,...(action==='status'?{}:{command_id:randomUUID(),binding:p.binding}),...extra};const result=await p.command({op:'host_browser',operation});if(result.status?.binding)p.binding=result.status.binding;return result;};
 if(!attach)return p;
 const status=await p.op('status');assert.equal(status.status.running,true,JSON.stringify(status));p.binding={...status.status.binding,attachment_id:randomUUID()};await p.op('attach');
 // The second attached viewer remains authorized while the first runs its
 // journey. Match the production viewer's status lease renewal, without capture.
 p.leaseTimer=setInterval(()=>{void p.op('status').catch(()=>{});},5000);
 return p;
}
async function decoded(page, label){
 let state;
 for(let tries=0;tries<150;tries++) {
  state=await page.evaluate(()=>{const frame=document.querySelector('.browser-next-mirror iframe');
   const body=frame?.contentDocument?.body;
   return {ready:['agent','human','private','watching'].includes(document.querySelector('.host-browser-viewer')?.dataset.state),
    document:!!body,heading:body?.querySelector('h1')?.textContent||'',text:body?.innerText?.slice(0,120)||'',url:document.querySelector('[aria-label="Website address"]')?.value||'',phase:window.mounted?.session.phase,issue:window.mounted?.session.issue,error:window.mounted?.session.lastError,rrweb:!!window.rrweb,streaming:window.mounted?.session.streaming};});
  if(state.ready&&state.document&&(!state.url.includes(cfg.site)||state.heading==='Synthetic voyage browser'))break;
  await new Promise(r=>setTimeout(r,200));
 }
 assert.ok(state.ready&&state.document&&(!state.url.includes(cfg.site)||state.heading==='Synthetic voyage browser'),JSON.stringify({label,dom_not_replayed:state}));
 evidence.steps.push({label,dom:state});save();
}
async function mode(page, label, expected, media=true){
 await page.getByRole('button',{name:label,exact:true}).click();
 await page.waitForFunction(expected=>document.querySelector('.host-browser-viewer')?.dataset.state===expected && !document.querySelector('.browser-primary').disabled,expected).catch(async error=>{
  const diagnostic=await page.evaluate(()=>({state:document.querySelector('.host-browser-viewer')?.dataset.state,status:document.querySelector('.browser-next-status')?.textContent,primary:document.querySelector('.browser-primary')?.textContent,enabled:!document.querySelector('.browser-primary')?.disabled,operations:window.operations?.slice(-8)}));
  evidence.mode_failure={label,expected,diagnostic};save();throw error;
 });
 if(media)await decoded(page,expected);
}
async function navigate(page,url){
 await page.getByRole('textbox',{name:'Website address',exact:true}).fill(url);
 await page.getByRole('button',{name:'Go to address',exact:true}).click();
 // Mounted receiver exposes its queue; native UI is checked through the replayed page and fixture visits.
 if(await page.evaluate(()=>!!window.mounted))await page.waitForFunction(()=>!window.mounted.session.sending && window.mounted.session.status?.running);
 await new Promise(r=>setTimeout(r,2500));
 await decoded(page,'after navigation');
}
async function more(page,label){
 const menu=page.locator('details');
 if(!await menu.getAttribute('open').then(v=>v!==null))await page.getByLabel('More browser options',{exact:true}).click();
 await page.getByRole('button',{name:label,exact:true}).click();
}
async function address(page,suffix){
 await page.waitForFunction(url=>document.querySelector('[aria-label="Website address"]').value===url,cfg.site+suffix);
 await decoded(page,'history '+suffix);
}
async function siteClasses(page){
 await navigate(page,cfg.site+'/site-classes');
 await page.waitForFunction(()=>{
  const session=window.mounted.session,doc=session.replayer?.iframe.contentDocument;
  const image=doc?.querySelector('#authenticated-asset');
  return doc?.querySelector('#shadow')?.shadowRoot?.textContent.includes('Open shadow content')
   && image?.complete && image.naturalWidth===40
   && [...session.frames.values()].some(f=>f.player.iframe.contentDocument?.body?.textContent.includes('Cross-origin child content'));
 });
 const facts=await page.evaluate(()=>{
  const session=window.mounted.session,doc=session.replayer.iframe.contentDocument;
  const image=doc.querySelector('#authenticated-asset');
  return {background:doc.defaultView.getComputedStyle(doc.body).backgroundColor,
   shadow:doc.querySelector('#shadow').shadowRoot.textContent,
   authenticated_image:image.complete&&image.naturalWidth===40,frames:session.frames.size};
 });
 assert.equal(facts.background,'rgb(17, 51, 85)','external stylesheet must be replayed');
 assert.equal(facts.authenticated_image,true,'authenticated image must be delivered to mirror');
 const started=performance.now();
 await page.frameLocator('.browser-next-mirror iframe').getByRole('button',{name:'Change page',exact:true}).click();
 await page.waitForFunction(()=>window.mounted.session.replayer.iframe.contentDocument.querySelector('#result').textContent==='Changed in task browser');
 const latency=performance.now()-started;
 const child=page.frameLocator('.browser-next-frame iframe').getByRole('button',{name:'Child action',exact:true});
 await child.click();
 await page.waitForFunction(()=>[...window.mounted.session.frames.values()].some(f=>f.player.iframe.contentDocument?.body?.textContent.includes('Child action observed')));
 evidence.steps.push({site_classes:facts,task_dom_action:true,cross_origin_action:true,input_to_visible_ms:Math.round(latency)});save();
}
async function layout(page,label){
 for(const [size,width,height] of [['desktop',1280,800],['mobile',390,844]]){
  await page.setViewportSize({width,height});
  await decoded(page,label+' '+size);
  assert.equal(await page.locator('.browser-next-chrome').evaluate(e=>getComputedStyle(e).display),'flex','production CSS must load');
  assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth && document.querySelector('.host-browser-viewer').scrollWidth<=innerWidth),'horizontal overflow');
  assert.ok(await page.getByRole('button',{name:'Browse privately',exact:true}).isVisible());
  assert.ok(await page.getByRole('button',{name:'Close viewer',exact:true}).isVisible());
  for(const name of ['Disconnect viewer','Close browser'])assert.equal(await page.getByRole('button',{name,exact:true}).isVisible(),false,'secondary controls belong under More');
  assert.equal(await page.getByRole('button',{name:'Start / Connect',exact:true}).count(),0);
  const screenshot=path.join(cfg.screenshots || path.dirname(cfg.evidence),path.basename(path.dirname(cfg.evidence))+'-'+label+'-'+size+'.png');
  await page.screenshot({path:screenshot});
  evidence.steps.push({layout:size,width,height,no_horizontal_overflow:true,screenshot:path.basename(screenshot),before_private_input:true});save();
 }
 await page.setViewportSize({width:1280,height:800});
}
async function sameBrowserWeb(item, browserId){
 const peer=await connect({...item},false),page=await browser.newPage(),trace=[],diagnostics=[];
 webDiagnostics.push(diagnostics);
 const safeCategory=value=>{
  if(value==null)return null;
  const categories=['private browser belongs to another principal','private owner unavailable',
   'attachment already live','socket already attached','stale browser binding',
   'browser viewer admission unavailable','browser operation refused or outcome unknown',
   'browser mirror authority changed','browser command identity conflict',
   'browser attachment missing','browser attachment authority mismatch'];
  return categories.includes(value)?value:'other refusal';
 };
 // Only public control metadata is retained. Never record request/response
 // bodies, input, mirror data, private page metadata, URLs or credentials.
 await page.exposeFunction('webExchange',async request=>{
  const operation=request.command.operation;
  if(diagnostics.length>=512)throw Error('fixture command bound exceeded');
  const diagnostic={action:operation?.action||request.command.op,input_kind:operation?.input?.type,
   requested_epochs:bindingEpochs(operation?.binding),
   mode:['agent','human','private'].includes(operation?.mode)?operation.mode:null,
   client_state:await page.evaluate(()=>{
    const s=window.mounted?.session;
    return {can_input:s?.canInput===true,busy:s?.busy===true,sending:s?.sending===true,
     streaming:s?.streaming===true,queue_length:s?.queue.length||0};
   })};
  diagnostics.push(diagnostic);
  const traced=['control','attach','detach'].includes(operation?.action);
  const entry=traced?{action:operation.action,
   mode:['agent','human','private'].includes(operation.mode)?operation.mode:null,
   ...(await page.evaluate(()=>{
    const session=window.mounted?.session,status=session?.status;
    const matches=!!status?.binding&&status.controller===status.binding.attachment_id;
    const phases=['idle','connecting','live','switching','recovering','error','closed'];
    const issues=['control-unknown','page-unavailable','status-unavailable','connection-unavailable'];
    return {current_attachment_matches_controller:matches,
     privateDisconnected:status?.mode==='private'&&!matches&&!session?.controls,
     controls:session?.controls===true,attached:session?.attached===true,
     phase:phases.includes(session?.phase)?session.phase:'other phase',
     issue:session?.issue==null?null:issues.includes(session.issue)?session.issue:'other issue'};
   }))}:null;
  if(entry){trace.push(entry);evidence.sameBrowserWebTrace=trace;save();}
  try{
   const response=await peer.exchange(request);
   diagnostic.refused=failed(response);
   diagnostic.outcome_unknown=response.outcome_unknown===true||response.result?.outcome_unknown===true;
   diagnostic.outer_refusal_category=publicRefusalCategory(response.error);
   diagnostic.inner_refusal_category=publicRefusalCategory(response.result?.error);
   diagnostic.reply_epochs=bindingEpochs(response.result?.result?.status?.binding);
   diagnostic.value_outcome=valueOutcome(response.result?.result?.value);
   if(entry){entry.outer_error=safeCategory(response.error);
    entry.inner_error=safeCategory(response.result?.error);
    entry.outcome_unknown=response.outcome_unknown===true||response.result?.outcome_unknown===true;
    const status=response.result?.result?.status;
    entry.returned_mode=['agent','human','private'].includes(status?.mode)?status.mode:null;
    entry.returned_attachment_matches_controller=!!status?.binding&&status.controller===status.binding.attachment_id;
    save();}
   if(response.result?.incarnation)peer.item.incarnation=response.result.incarnation;
   return response;
  }catch(error){
   diagnostic.transport_failed=true;
   if(entry){entry.transport_error='transport rejected; response unavailable';save();}
   throw error;
  }
 });
 await page.goto(cfg.site+'/receiver');
 await page.evaluate(async item=>{
  const {mountHostBrowser}=await import('/web/resources/js/host-browser.js');
  window.mounted=mountHostBrowser(document.querySelector('main'),{client:{exchange:window.webExchange},sessionId:item.session,incarnation:item.incarnation,context:()=>({revision:0})});
 },peer.item);
 await page.waitForFunction(()=>window.mounted.session.attached&&!window.mounted.session.busy);
 await decoded(page,'same-browser Web adapter');
 assert.equal(await page.evaluate(()=>window.mounted.session.status.binding.browser_id),browserId);
 return {page,trace,peer};
}
async function privateExcluded(page){
 await page.waitForFunction(()=>window.mounted
  ?window.mounted.session.status?.mode==='private'&&!window.mounted.session.controls&&!window.mounted.session.streaming
  :window.fixtureBrowser?.mode==='private'&&!window.fixtureBrowser.controls&&!document.querySelector('.browser-next-mirror iframe'));
 assert.equal(await page.getByRole('textbox',{name:'Website address',exact:true}).inputValue(),'');
 const values=await page.evaluate(()=>[...document.querySelectorAll('iframe')].map(frame=>{
  const doc=frame.contentDocument;return [doc?.body?.textContent||'',...[...(doc?.querySelectorAll('input,textarea')||[])].map(e=>e.value)].join(' ');
 }).join(' '));
 assert.equal(values.includes('SYNTHETIC_DUAL_PRIVATE_333')||values.includes('SYNTHETIC_BROWSER_FILE_333'),false,'other viewer must not expose private values');
}
async function native(item,observer,other){
 const page=await browser.newPage();
 await nativeMetadata(page);
 await page.goto(new URL('file://'+item.launcher).href);
 await page.getByRole('button',{name:'Browse privately',exact:true}).waitFor();
 assert.equal(new URL(page.url()).hash,'','bootstrap must remove launch secret from history');
 await decoded(page,'native '+item.native_route);
 const observed=await observer.op('status');
 if(observed.status.binding.attachment_id==='00000000-0000-0000-0000-000000000000'){
  observer.binding={...observed.status.binding,attachment_id:randomUUID()};await observer.op('attach');
 }
 const joint=item.native_entry==='tui-f6'?await sameBrowserWeb(item,(await observer.op('status')).status.binding.browser_id):null;
 if(joint)await viewerCapacity(item);
 evidence.nativeStage='before private '+item.native_route;save();
 await mode(page,'Browse privately','private');
 if(joint)await privateExcluded(joint.page);
 await clientBreadth(page,item.native_route,{observer,other});
 if(joint)await privateExcluded(joint.page);
 evidence.nativeStage='before navigate '+item.native_route;save();
 await navigate(page,cfg.site+'/native-'+item.native_route);
 if(joint){
  await page.getByRole('textbox',{name:'Text for browser',exact:true}).fill('SYNTHETIC_DUAL_PRIVATE_333');
  await page.getByRole('button',{name:'Send text to browser',exact:true}).click();
  await privateExcluded(joint.page);
  await page.getByRole('button',{name:'Close viewer',exact:true}).click();
  await page.waitForFunction(()=>!document.querySelector('.browser-next-mirror'));
  assert.equal((await observer.op('status')).status.mode,'private','private detach must not resume agent');
  await mode(joint.page,'Browse privately','private',false);
  const reclaim=joint.trace.slice(-2);
  assert.deepEqual(reclaim.map(step=>step.action),['detach','attach'],'explicit reclaim must retire stale attachment and acknowledge a fresh join');
  assert.ok(reclaim.every(step=>step.returned_mode==='private'&&!step.outer_error&&!step.inner_error&&!step.outcome_unknown),'reclaim must remain private and confirmed');
  assert.equal(reclaim.at(-1).returned_attachment_matches_controller,true,'fresh private attachment must own control');
  assert.equal(joint.trace.some(step=>step.action==='control'&&step.mode==='human'),false,'private reclaim must never publish human mode');
  await clientBreadth(joint.page,'web-access-file',{observer,other,peer:joint.peer});
  await mode(joint.page,'Continue agent','agent');
  await joint.page.getByRole('button',{name:'Close viewer',exact:true}).click();
  await joint.page.waitForFunction(()=>!document.querySelector('.browser-next-mirror'));
  await joint.page.close();
  evidence.steps.push({same_browser_two_clients:true,native_entry:'tui-f6',private_excluded:true,disconnect_stayed_private:true,explicit_same_principal_reclaim:true});save();
 }else{
  evidence.nativeStage='before agent '+item.native_route;save();
  await mode(page,'Continue agent','agent');
 }
 evidence.nativeStage='before agent replay '+item.native_route;save();
 if(!joint)await decoded(page,'native return '+item.native_route);
 evidence.nativeStage='before close '+item.native_route;save();
 if(!joint){await page.getByRole('button',{name:'Close viewer',exact:true}).click();
 await page.waitForFunction(()=>!document.querySelector('.browser-next-mirror'));}
 await new Promise(r=>setTimeout(r,300));
 assert.equal((await observer.op('status')).status.running,true,'viewer disconnect must not close host');
 evidence.steps.push({native_route:item.native_route,authenticated:true,disconnect_preserved_host:true});save();
 await page.close();
}
async function suspendedNative(item){
 const page=await browser.newPage();
 await page.addInitScript(()=>{
  window.operations=[];
  const fetchNative=window.fetch;window.fetch=async(...args)=>{
   const response=await fetchNative(...args);
   if(args[0]==='/operation'){
    const op=JSON.parse(args[1].body), reply=await response.clone().json();
    window.operations.push({action:op.action,command_id:op.command_id,incarnation:op.incarnation,
     context:reply.context,running:reply.result?.status?.running,owner:reply.result?.status?.binding?.incarnation,http:response.status});
   }return response;
  };
 });
 let closed=false,primaryFailure=null;
 try{
  await page.goto(new URL('file://'+item.launcher).href);
  await page.waitForFunction(()=>window.operations.some(o=>o.action==='start'&&o.running===true));
  const ops=await page.evaluate(()=>window.operations);
  assert.equal(ops[0].action,'status');assert.equal(ops[0].running,false);
  assert.notEqual(ops[0].context.incarnation,item.incarnation);
  assert.equal(ops[1].action,'start');assert.equal(ops[1].incarnation,ops[0].context.incarnation);
  assert.equal(ops[1].running,true);
  assert.equal(ops.filter(o=>o.action==='start').length,1,'Start must not replay');
  evidence.steps.push({native_route:item.native_route,real_suspended_owner:item.incarnation,operations:ops});save();
  await mode(page,'Browse privately','private',false);
  await navigate(page,cfg.site+'/suspended-native-'+item.native_route);
  await decoded(page,'suspended native '+item.native_route);
 }catch(error){
  primaryFailure=error;
  evidence.suspendedNativeFailure={session:item.session,native_route:item.native_route,
   name:error.name,message:String(error.message).slice(0,2048),
   operations:await page.evaluate(()=>window.operations?.slice(-12)||[]).catch(()=>[])};
  save();throw error;
 }finally{
  try{
   if(!await page.getByRole('button',{name:'Close browser',exact:true,includeHidden:true}).isEnabled())
    throw Error('close_control_unavailable; browser cleanup remains unconfirmed');
   await more(page,'Close browser');
   await page.waitForFunction(()=>window.operations.some(o=>o.action==='close'&&o.running===false));
   closed=true;evidence.cleanup[item.session]={closed};save();
  }catch(error){
   const preservePrimary=!!primaryFailure;
   primaryFailure ||= error;
   evidence.cleanup[item.session]={closed:false,name:error.name,message:String(error.message).slice(0,2048)};
   save();if(!preservePrimary)throw error;
  }finally{
   try{await page.close();}
   catch(error){
    evidence.cleanup[item.session]={...evidence.cleanup[item.session],viewer_closed:false,
     viewer_close_error:{name:error.name,message:String(error.message).slice(0,2048)}};
    save();if(!primaryFailure)throw error;
   }
  }
 }
}
async function suspendedWeb(item){
 const p=await connect(item,false),page=await browser.newPage(),trace=[];
 await page.exposeFunction('webExchange',async request=>{
  const response=await p.exchange(request),command=request.command,reply=response.result;
  trace.push({op:command.op,action:command.operation?.action,command_id:command.operation?.command_id,
   requested_owner:command.incarnation,owner:reply?.incarnation,prepared:reply?.result?.status==='prepared',
   not_dispatched:reply?.result?.not_dispatched,revision:reply?.result?.revision,running:reply?.result?.status?.running,
   error:response.error!=null||reply?.error!=null});
  if(reply?.incarnation)p.item.incarnation=reply.incarnation;
  if(reply?.result?.status?.binding)p.binding=reply.result.status.binding;
  evidence.web_trace=trace;save();return response;
 });
 await page.goto(cfg.site+'/receiver');
 await page.evaluate(async item=>{const {mountHostBrowser}=await import('/web/resources/js/host-browser.js');window.mounted=mountHostBrowser(document.querySelector('main'),{client:{exchange:window.webExchange},sessionId:item.session,incarnation:item.incarnation,context:()=>({revision:0})});},item);
 await page.waitForFunction(()=>window.mounted.session.attached&&!window.mounted.session.busy);
 assert.deepEqual(trace.slice(0,4).map(t=>t.action||t.op),['status','snapshot','status','start']);
 assert.equal(trace[0].prepared,true);assert.equal(trace[0].not_dispatched,true);
 assert.notEqual(trace[0].requested_owner,trace[0].owner);
 for(const t of trace.slice(1,4))assert.equal(t.requested_owner,trace[0].owner);
 assert.equal(trace[2].running,false);assert.equal(trace[3].running,true);
 assert.equal(trace.filter(t=>t.action==='start').length,1);
 evidence.steps.push({web:item.label,preparation:trace.slice(0,4)});save();
 await mode(page,'Browse privately','private',false);
 await navigate(page,cfg.site+'/suspended-web-'+item.label);
 await decoded(page,'suspended Web '+item.label);
 await more(page,'Close browser');
 await page.waitForFunction(()=>window.mounted.session.status?.running===false&&!window.mounted.session.busy);
 evidence.cleanup[item.session]={closed:true};save();
 await page.evaluate(()=>window.mounted.dispose());await page.close();
}
async function adverseProgress(page,label){
 try{
  const view=await page.evaluate(()=>{
   const s=window.mounted?.session,status=s?.status||window.fixtureBrowser;
   const frame=document.querySelector('.browser-next-mirror iframe'),raw=frame?.contentDocument?.querySelector('#count')?.textContent;
   const count=/^[0-8]$/.test(raw||'')?Number(raw):raw==null?'absent':'other';
   const phases=['idle','connecting','live','switching','recovering','error','closed','waiting-page','stopped','unavailable','disconnected','needs-review'];
   const issues=['connect-unknown','control-unknown','input-unknown','close-unknown','page-unavailable','status-unavailable','connection-unavailable'];
   return {replayed_count:count,frame_present:!!frame,running:status?.running===true,
    mode:['agent','human','private'].includes(status?.mode)?status.mode:'unknown',
    phase:phases.includes(s?.phase)?s.phase:s?'other':'native',issue:s?.issue==null?null:issues.includes(s.issue)?s.issue:'other',
    streaming:s?s.streaming===true:!!frame,can_input:s?s.canInput===true:document.querySelector('.browser-next-frames')?.dataset.control==='true',
    sending:s?.sending===true,queue_length:s?.queue.length||0,effects_pending:window.fixtureEffectsPending||0};
  });
  const response=await fetch(cfg.site+'/fixture-counter?label='+label,{signal:AbortSignal.timeout(2000)});
  assert.equal(response.status,200);const observed=await response.json();
  return {...view,task_reported_count:Number.isInteger(observed.count)&&observed.count>=0&&observed.count<=8?observed.count:'other'};
 }catch{return {state_unavailable:true};}
}
async function originalReceiptRead(peer,owner,commandId){
 const response=await peer.exchange({protocol:1,command:{op:'host_browser',session_id:peer.item.session,incarnation:owner,
  operation:{action:'receipt',command_id:commandId}}});
 if(failed(response)){
  const category=response.error||response.result?.error;
  assert.ok(['browser owner is suspended','stale runtime incarnation'].includes(category),'original-owner receipt read must have a definite suspension/fence refusal');
  assert.equal(response.outcome_unknown===true||response.result?.outcome_unknown===true,false);
  return {original_owner_read:'known_owner_unavailable',no_preparation_or_retry:true};
 }
 const receipt=response.result?.result?.receipt;
 assert.equal(receipt?.command_id,commandId);assert.equal(receipt.state,'unknown');
 return {original_owner_read:'unknown_receipt_observed',no_preparation_or_retry:true};
}
async function adverseClients(){
 const nativeItem=cfg.sessions[0],otherItem=cfg.sessions[1],nativePage=await browser.newPage(),otherNativePage=await browser.newPage();
 const gate=mirrorGate();let slow,other,withheld=false;
 try{
  await nativeMetadata(nativePage);
  await nativePage.goto(new URL('file://'+nativeItem.launcher).href);
  await nativePage.getByRole('button',{name:'Browse privately',exact:true}).waitFor();
  await decoded(nativePage,'adverse native launcher');
  await navigate(nativePage,cfg.site+'/ui/adverse-native');
  const fresh=await nativePage.evaluate(()=>({context:window.fixtureContext,running:window.fixtureBrowser?.running,
   binding_owner:window.fixtureBrowser?.binding?.incarnation}));
  assert.equal(fresh.running,true);assert.ok(Number.isSafeInteger(fresh.context?.revision));
  assert.equal(fresh.context.incarnation,fresh.binding_owner,'native actual owner must match the acknowledged browser');
  evidence.adverseSetup={native_running_observed:true,native_owner_from_actual_reply:true,
   native_owner_changed_since_saved:fresh.context.incarnation!==nativeItem.incarnation};
  evidence.adverseStage='opening independent native browser';save();
  // CLI launcher creation has not opened a browser. Bootstrap the actual
  // second native viewer before mounting a Web observer of that browser.
  await nativeMetadata(otherNativePage);
  await otherNativePage.goto(new URL('file://'+otherItem.launcher).href);
  await otherNativePage.getByRole('button',{name:'Browse privately',exact:true}).waitFor();
  await decoded(otherNativePage,'adverse independent native launcher');
  const freshOther=await otherNativePage.evaluate(()=>({context:window.fixtureContext,running:window.fixtureBrowser?.running,
   binding_owner:window.fixtureBrowser?.binding?.incarnation,starts:window.fixtureOperations.filter(o=>o.action==='start').length}));
  assert.equal(freshOther.running,true);assert.ok(Number.isSafeInteger(freshOther.context?.revision));
  assert.equal(freshOther.context.incarnation,freshOther.binding_owner);assert.equal(freshOther.starts,1,'independent browser starts once');
  Object.assign(evidence.adverseSetup,{other_native_running_observed:true,other_owner_from_actual_reply:true,other_browser_started_once:true});
  evidence.adverseStage='mounting slow Web observer with fresh snapshot';save();
  slow=await mountedWeb({...nativeItem,...fresh.context},{gate,refresh:true});
  evidence.adverseSetup.slow_web_attached=true;evidence.adverseStage='mounting independent Web observer with fresh snapshot';save();
  other=await mountedWeb({...otherItem,...freshOther.context},{refresh:true});
  evidence.adverseSetup.other_web_attached=true;evidence.adverseStage='holding confirmed mirror replies';save();
  const nativeFrame=nativePage.frameLocator('.browser-next-mirror iframe');
  gate.holding=true;
  const entered=Date.now()+5000;while(!gate.entered&&Date.now()<entered)await pause(50);
  assert.ok(gate.entered,'mirror gate must hold an actual read response');
  evidence.adverseStage='native counter input readiness';save();await readyInput(nativePage);
  evidence.adverseStage='native counter click';save();
  await nativeFrame.getByRole('button',{name:'Increment task counter',exact:true}).click();
  await readyInput(nativePage);evidence.adverseStage='native counter replay wait';save();
  await nativeFrame.locator('#count').filter({hasText:'1'}).waitFor();
  evidence.adverseProgress={native:await adverseProgress(nativePage,'adverse-native')};
  evidence.adverseStage='other Web navigation readiness';save();await readyInput(other.page,{claim:true});
  evidence.adverseStage='other Web navigation';save();
  await navigate(other.page,cfg.site+'/ui/adverse-other');
  await readyInput(other.page);evidence.adverseStage='other Web counter click';save();
  const otherFrame=other.page.frameLocator('.browser-next-mirror iframe');
  await otherFrame.getByRole('button',{name:'Increment task counter',exact:true}).click();
  await readyInput(other.page);evidence.adverseStage='other Web counter replay wait';
  evidence.adverseProgress.other_web=await adverseProgress(other.page,'adverse-other');save();
  await otherFrame.locator('#count').filter({hasText:'1'}).waitFor();
  evidence.adverseStage='stalled observer timeout presentation';save();
  await slow.page.waitForFunction(()=>window.mounted.session.issue==='page-unavailable'&&!window.mounted.session.polling,null,{timeout:32000});
  assert.equal(await slow.page.locator('.browser-next-recovery').isVisible(),true);
  assert.equal(await slow.page.evaluate(()=>window.mounted.session.queue.length),0,'stalled observer must not collect queued actions');
  assert.equal(await slow.page.evaluate(()=>window.mounted.session.canInput),false);
  assert.ok(gate.peak<=2,'read timeout must bound overlapping held replies');
  evidence.adverseSetup.stalled_timeout_observed=true;
  evidence.adverseStage='explicit slow observer read recovery';save();
  const recovery=slow.page.getByRole('button',{name:'Check browser status',exact:true}).click();
  gate.release();await recovery;
  await slow.page.waitForFunction(()=>window.mounted.session.streaming&&!window.mounted.session.issue);
  evidence.adverseStage='recovered slow observer counter replay wait';
  evidence.adverseProgress.slow_web=await adverseProgress(slow.page,'adverse-native');save();
  await slow.page.frameLocator('.browser-next-mirror iframe').locator('#count').filter({hasText:'1'}).waitFor();
  assert.equal(slow.trace.some(t=>t.action==='input'),false,'stalled observer must not replay an effect');
  evidence.steps.push({stalled_real_mirror_reply:true,production_read_timeout_ms:25000,
   held_replies_peak:gate.peak,client_recovery_visible:true,explicit_read_recovery:true,
   native_controller_progress:true,other_voyage_progress:true,no_observer_effect_replay:true});save();

  // Move control explicitly before killing the one selected worker. The Web
  // owner's real Close browser button can now expose cleanup uncertainty.
  evidence.adverseStage='explicit private control before worker crash';save();
  await mode(nativePage,'Continue agent','agent');
  await mode(slow.page,'Browse privately','private');
  await privateExcluded(nativePage);
  await pause(500);
  await slow.page.waitForFunction(()=>window.mounted.session.canInput&&!window.mounted.session.sending&&window.mounted.session.queue.length===0);
  evidence.adverseStage='owned worker crash and evidence withholding';save();
  await fixtureControl('crash-worker',nativeItem);
  await fixtureControl('withhold-cleanup',nativeItem);withheld=true;
  evidence.adverseStage='first explicit close with unavailable evidence';save();
  await more(slow.page,'Close browser');
  await slow.page.waitForFunction(()=>window.mounted.session.issue==='close-unknown'&&!window.mounted.session.busy);
  assert.equal(await slow.page.locator('.browser-next-recovery').isVisible(),true);
  const first=slow.trace.filter(t=>t.action==='close');assert.equal(first.length,1);
  assert.equal(first[0].refused,true);assert.equal(first[0].outcome_unknown,true);
  const retained=await fixtureControl('cleanup-state',nativeItem);
  evidence.adverseCleanup={retained};evidence.adverseStage='retained cleanup obligation assertions';save();
  assert.equal(retained.live_owned_browser_processes,0);assert.equal(retained.owned_browser_zombies,0);
  assert.equal(retained.scratch_removed,true);assert.equal(retained.cleanup_evidence_available,false);
  assert.equal(retained.worker_lock_retained,true);assert.equal(retained.worker_lock_unchanged,true);assert.equal(retained.capacity_slots_retained,1);
  await pause(800);
  assert.equal(slow.trace.filter(t=>t.action==='close').length,1,'uncertain close must not retry automatically');
  await otherFrame.getByRole('button',{name:'Increment task counter',exact:true}).click();
  await otherFrame.locator('#count').filter({hasText:'2'}).waitFor();
  evidence.adverseStage='same evidence restoration before new explicit cleanup';save();
  await fixtureControl('restore-cleanup',nativeItem);withheld=false;
  // A new explicit cleanup decision after observing the retained uncertainty.
  // The original command stays unknown; it is never resent or relabelled.
  evidence.adverseStage='second explicit close after evidence restoration';save();
  await more(slow.page,'Close browser');
  await slow.page.waitForFunction(()=>window.mounted.session.status.running===false&&!window.mounted.session.busy);
  const closes=slow.trace.filter(t=>t.action==='close');assert.equal(closes.length,2);
  assert.notEqual(closes[0].command_id,closes[1].command_id);assert.equal(closes[1].refused,false);
  evidence.adverseStage='original unknown receipt observation';save();
  const receipt=await originalReceiptRead(slow.peer,fresh.context.incarnation,closes[0].command_id);
  const pinned=await fixtureControl('pin-saved-receipt',nativeItem,{command_id:closes[0].command_id});
  assert.equal(pinned.state,'unknown');assert.equal(pinned.original_principal_matches,true);assert.equal(pinned.read_only_storage,true);
  const cleaned=await fixtureControl('cleanup-state',nativeItem);
  evidence.adverseCleanup.cleaned=cleaned;evidence.adverseCleanup.original_close_still_unknown=pinned.state==='unknown';
  evidence.adverseCleanup.initial_receipt_read=receipt;
  evidence.adverseStage='observed cleanup assertions';save();
  assert.equal(cleaned.capacity_slots_retained,0);assert.equal(cleaned.guardian_observed,true);
  assert.equal(cleaned.live_owned_browser_processes,0);assert.equal(cleaned.owned_browser_zombies,0);assert.equal(cleaned.scratch_removed,true);
  assert.equal(cleaned.cleanup_evidence_available,true);assert.equal(cleaned.worker_lock_unchanged,true);
  assert.equal(cleaned.worker_lock_retained,true,'crash lock remains as recovery evidence');
  assert.equal(cleaned.external_actions_reconciled,false);
  evidence.adverseStage='stopped cleanup presentation and safe explicit admission';save();
  assert.equal(await slow.page.locator('.browser-next-recovery').isVisible(),true);
  assert.equal(await slow.page.locator('.browser-next-status').textContent(),'Browser stopped');
  const recoveryButton=slow.page.locator('.browser-next-recovery button');
  assert.equal(await recoveryButton.textContent(),'Start browser');
  assert.equal(await recoveryButton.getAttribute('aria-label'),await recoveryButton.textContent(),'accessible action must match displayed intent');
  const beforeStart=slow.trace.filter(t=>t.action==='start').length;
  // This is one new explicit admission request, not replay of either close or
  // a site effect. The retained crash lock must refuse a replacement worker.
  await recoveryButton.click();
  await slow.page.waitForFunction(()=>window.mounted.session.issue==='connect-unknown'&&!window.mounted.session.busy);
  const starts=slow.trace.filter(t=>t.action==='start');assert.equal(starts.length,beforeStart+1);
  assert.equal(starts.at(-1).refused,true,'unreconciled crash must refuse new browser admission');
  assert.equal(await slow.page.evaluate(()=>window.mounted.session.status.running),false);
  assert.equal(await slow.page.evaluate(()=>window.mounted.session.canInput),false);
  assert.equal(await slow.page.locator('.browser-next-recovery').isVisible(),true);
  await pause(800);assert.equal(slow.trace.filter(t=>t.action==='start').length,beforeStart+1,'refused admission must not retry itself');
  const refusedAdmission=await fixtureControl('cleanup-state',nativeItem);
  evidence.adverseCleanup.refused_admission=refusedAdmission;save();
  assert.equal(refusedAdmission.worker_lock_unchanged,true);assert.equal(refusedAdmission.capacity_slots_retained,0);
  assert.equal(refusedAdmission.live_owned_browser_processes,0);assert.equal(refusedAdmission.owned_browser_zombies,0);
  assert.equal(refusedAdmission.guardian_observed,true);assert.equal(refusedAdmission.external_actions_reconciled,false);
  evidence.adverseStage='original-owner and saved exact receipt observation after refused admission';save();
  const original=await originalReceiptRead(slow.peer,fresh.context.incarnation,closes[0].command_id);
  const unchanged=await fixtureControl('check-saved-receipt',nativeItem,{command_id:closes[0].command_id});
  assert.equal(unchanged.state,'unknown');assert.equal(unchanged.exact_receipt_unchanged,true);
  assert.equal(unchanged.original_principal_matches,true);assert.equal(unchanged.read_only_storage,true);
  evidence.adverseCleanup.final_receipt_read=original;evidence.adverseCleanup.saved_exact_receipt=unchanged;save();
  evidence.steps.push({actual_worker_crash:true,cleanup_evidence_unavailable:retained,
   explicit_cleanup_after_same_evidence_restored:true,original_close_receipt:'unknown',
   no_automatic_close_replay:true,independent_voyage_continued:true,observed_cleanup:cleaned,
   replacement_admission_refused:true,no_automatic_start_retry:true});
  evidence.cleanup[nativeItem.session]={closed:true,guardian:cleaned};save();
  await more(other.page,'Close browser');
  await other.page.waitForFunction(()=>window.mounted.session.status.running===false&&!window.mounted.session.busy);
  evidence.cleanup[otherItem.session]={closed:true};save();
 }catch(error){
  evidence.adverseProgress={...evidence.adverseProgress,
   native:await adverseProgress(nativePage,'adverse-native'),
   ...(slow?{slow_web:await adverseProgress(slow.page,'adverse-native')}:{}),
   ...(other?{other_web:await adverseProgress(other.page,'adverse-other')}:{}),
   other_native:await adverseProgress(otherNativePage,'adverse-other')};
  retainWebDiagnostics();save();throw error;
 }finally{
  gate.release();
  if(withheld)await fixtureControl('restore-cleanup',nativeItem);
  if(slow){await slow.page.evaluate(()=>window.mounted.dispose());await slow.page.close();}
  if(other){await other.page.evaluate(()=>window.mounted.dispose());await other.page.close();}
  await nativePage.close();
  await otherNativePage.close();
 }
}
try{
 browser=await chromium.launch({executablePath:cfg.chromium,headless:true,chromiumSandbox:true,args:['--disable-background-networking']});
 if(cfg.mode==='adverse')await adverseClients();
 else if(cfg.mode==='suspended-native'){for(const item of cfg.sessions)await suspendedNative(item);}
 else if(cfg.mode==='suspended-web'){for(const item of cfg.sessions)await suspendedWeb(item);}
 else {
 const a=await connect(cfg.sessions[0]), b=await connect(cfg.sessions[1]);assert.notEqual(a.binding.browser_id,b.binding.browser_id);
 // The fixture only adapts transport; UI, replay and operation sequencing are production code.
 for(const p of [a,b]){
  const page=await browser.newPage();p.page=page;const operations=[];
  await page.exposeFunction('hostTransport',async operation=>{
   const entry={action:operation.action,sequence:operation.sequence,input_kind:operation.input?.type};
   operations.push(entry);
   try{const result=await p.command({op:'host_browser',operation});if(result.status?.binding)p.binding=result.status.binding;entry.reply={running:result.status?.running,reset:result.value?.reset,encoding:result.value?.encoding,size:result.value?.data_base64?.length};evidence.operations=operations;save();return result;}
   catch(error){entry.error=String(error).slice(0,500);evidence.operations=operations;save();throw error;}
  });
  await page.goto(cfg.site+'/receiver');
  await page.evaluate(async item=>{const {mountBrowserViewer}=await import('/viewer.mjs');window.mounted=mountBrowserViewer(document.querySelector('main'),{context:()=>({incarnation:item.incarnation,revision:0}),transport:window.hostTransport});},p.item);
  await decoded(page,'mounted automatic '+p.item.label);
  await new Promise(r=>setTimeout(r,2200));
  assert.equal(operations.filter(o=>o.action==='start').length,0,'running host must not restart');
  assert.equal(operations.filter(o=>o.action==='mirror').length>0,true,'mount must request the live page');
  await layout(page,p.item.label);
  await navigate(page,cfg.site+'/ordinary-first-action');
  await page.waitForFunction(()=>window.mounted.session.status?.mode==='human');
  evidence.steps.push({ordinary_navigation_claimed_human:true});save();
  await siteClasses(page);
  await mode(page,'Browse privately','private');
  await navigate(page,cfg.site+'/history-one');
  await page.getByRole('group',{name:'Browser tabs',exact:true}).getByRole('button',{name:'Synthetic /history-one',exact:true}).waitFor();
  await navigate(page,cfg.site+'/history-two');
  await page.getByRole('button',{name:'Back',exact:true}).click();
  await address(page,'/history-one');
  await page.getByRole('button',{name:'Forward',exact:true}).click();
  await address(page,'/history-two');
  // Exercise the real stop action as well; stale loading metadata is recorded, not hidden.
  if(await page.getByRole('button',{name:'Stop loading',exact:true}).isVisible()){
   evidence.steps.push({loading_after_forward:true,action:'stop_loading'});save();
   await page.getByRole('button',{name:'Stop loading',exact:true}).click();
  }
  await page.getByRole('button',{name:'Reload',exact:true}).click();
  await page.waitForFunction(()=>!window.mounted.session.sending && window.mounted.session.queue.length===0);
  await decoded(page,'history reload');
  await navigate(page,cfg.site+'/private');
  await page.getByRole('textbox',{name:'Text for browser',exact:true}).fill('SYNTHETIC_PRIVATE_INPUT_333');
  await page.getByRole('button',{name:'Send text to browser',exact:true}).click();
  await page.waitForFunction(()=>!window.mounted.session.sending);
  assert.equal(await page.evaluate(()=>window.mounted.session.status?.mode),'private');
  assert.equal((await (p===a?b:a).op('status')).status.mode,'agent');
  evidence.steps.push({ime_private_sent:true,other_voyage_remained_agent:true});save();
  await page.getByRole('textbox',{name:'Website address',exact:true}).fill(cfg.site+'/modal');
  await page.getByRole('button',{name:'Go to address',exact:true}).click();
  await page.getByRole('region',{name:'Website dialog',exact:true}).waitFor();
  await page.getByRole('textbox',{name:'Website dialog response',exact:true}).fill('synthetic response');
  await page.getByRole('button',{name:'Accept dialog',exact:true}).click();
  try {await page.getByRole('region',{name:'Website dialog',exact:true}).waitFor({state:'hidden'});}
  catch(error){evidence.steps.push({dialog_failure:p.item.label,state:await page.evaluate(()=>{
   const s=window.mounted.session;return {phase:s.phase,issue:s.issue,busy:s.busy,sending:s.sending,streaming:s.streaming,
    can_input:s.canInput,queue:s.queue.length,sequence:s.sequence,dialog:s.status?.dialog,mode:s.status?.mode};}),
   recent_operations:operations.slice(-12)});save();throw error;}
  await mode(page,'Continue agent','agent');
  assert.equal(operations.filter(o=>o.action==='start').length,0,'no incidental host restart');
  assert.equal(operations.filter(o=>o.action==='attach').length,0,'reuse pre-attached fixture without duplicate attachment');
  await page.getByRole('button',{name:'Close viewer',exact:true}).click();
  await page.waitForFunction(()=>!document.querySelector('.browser-next-mirror'));
  await new Promise(r=>setTimeout(r,300));
  assert.equal((await p.op('status')).status.running,true,'close viewer must preserve host');
  evidence.steps.push({mounted:p.item.label,automatic_connections:1,history:true,modal:true,ime:true,close_viewer_preserved_host:true});save();
  await page.close();
 }
 await native(a.item,a,b);await native(b.item,b,a);
 }
 evidence.actions='passed';
}catch(e){evidence.failure={name:e.name,message:String(e.message).replace(/(?:file|https?):\/\/[^\s\"']+/g,'[private URL omitted]')};retainWebDiagnostics();process.exitCode=1;}
finally{
 for(const p of peers)clearInterval(p.leaseTimer);
 for(const p of peers){try{evidence.pre_teardown.push({session:p.item.session,...await p.op('status')});}catch(e){evidence.pre_teardown.push({error:String(e)});} }save();
 for(const p of peers){try{if(!p.binding||evidence.cleanup[p.item.session]?.closed){p.ws.close();continue;}const current=await p.op('status');if(current.status.running){if(!p.binding?.attachment_id||p.binding.attachment_id==='00000000-0000-0000-0000-000000000000'){p.binding={...current.status.binding,attachment_id:randomUUID()};await p.op('attach');}await p.op('close');}const s=await p.op('status');assert.equal(s.status.running,false);evidence.cleanup[p.item.session]=s;}catch(e){evidence.cleanup[p.item.session]={error:String(e)};process.exitCode=1;}p.ws.close();}
 if(browser)await browser.close();if(!process.exitCode&&evidence.actions==='passed')evidence.journey='passed';
 if(process.exitCode)retainWebDiagnostics();save();
}
// Ensure bounded completion even if a failed socket retained an unref'd timer.
setTimeout(()=>process.exit(process.exitCode||0),50);
