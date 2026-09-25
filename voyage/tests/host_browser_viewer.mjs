// Production shared viewer and native Helm launcher over real authenticated Vessel sockets.
import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
import {randomUUID} from 'node:crypto';
import assert from 'node:assert/strict';
const require=createRequire(import.meta.url);
const cfg=JSON.parse(fs.readFileSync(process.argv[2]));
const WebSocket=require(cfg.ws);
const {chromium}=require(cfg.playwright);
const peers=[], evidence={steps:[],pre_teardown:[],cleanup:{}};
let browser;
const save=()=>fs.writeFileSync(cfg.evidence,JSON.stringify(evidence,null,2));
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
 const status=await p.op('status');assert.equal(status.status.running,true,JSON.stringify(status));p.binding={...status.status.binding,attachment_id:randomUUID()};await p.op('attach');return p;
}
async function decoded(page, label){
 let state;
 for(let tries=0;tries<150;tries++) {
  state=await page.evaluate(()=>{const frame=document.querySelector('.browser-next-mirror iframe');
   const body=frame?.contentDocument?.body;
   return {ready:document.querySelector('.host-browser-viewer')?.dataset.state==='live'||['agent','private','watching'].includes(document.querySelector('.host-browser-viewer')?.dataset.state),
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
async function layout(page,label){
 for(const [size,width,height] of [['desktop',1280,800],['mobile',390,844]]){
  await page.setViewportSize({width,height});
  await decoded(page,label+' '+size);
  assert.equal(await page.locator('.browser-next-chrome').evaluate(e=>getComputedStyle(e).display),'flex','production CSS must load');
  assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth && document.querySelector('.host-browser-viewer').scrollWidth<=innerWidth),'horizontal overflow');
  assert.ok(await page.getByRole('button',{name:'Take control privately',exact:true}).isVisible());
  assert.ok(await page.getByRole('button',{name:'Close viewer',exact:true}).isVisible());
  for(const name of ['Disconnect viewer','Close browser'])assert.equal(await page.getByRole('button',{name,exact:true}).isVisible(),false,'secondary controls belong under More');
  assert.equal(await page.getByRole('button',{name:'Start / Connect',exact:true}).count(),0);
  const screenshot=path.join(cfg.screenshots || path.dirname(cfg.evidence),path.basename(path.dirname(cfg.evidence))+'-'+label+'-'+size+'.png');
  await page.screenshot({path:screenshot});
  evidence.steps.push({layout:size,width,height,no_horizontal_overflow:true,screenshot:path.basename(screenshot),before_private_input:true});save();
 }
 await page.setViewportSize({width:1280,height:800});
}
async function native(item,observer){
 const page=await browser.newPage();
 await page.goto(new URL('file://'+item.launcher).href);
 await page.getByRole('button',{name:'Take control privately',exact:true}).waitFor();
 assert.equal(new URL(page.url()).hash,'','bootstrap must remove launch secret from history');
 await decoded(page,'native '+item.native_route);
 evidence.nativeStage='before private '+item.native_route;save();
 await mode(page,'Take control privately','private');
 evidence.nativeStage='before navigate '+item.native_route;save();
 await navigate(page,cfg.site+'/native-'+item.native_route);
 evidence.nativeStage='before agent '+item.native_route;save();
 await mode(page,'Return to agent','agent');
 evidence.nativeStage='before agent replay '+item.native_route;save();
 await decoded(page,'native return '+item.native_route);
 evidence.nativeStage='before close '+item.native_route;save();
 await page.getByRole('button',{name:'Close viewer',exact:true}).click();
 await page.waitForFunction(()=>!document.querySelector('.browser-next-mirror'));
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
 let closed=false;
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
  await mode(page,'Take control privately','private',false);
  await navigate(page,cfg.site+'/suspended-native-'+item.native_route);
  await decoded(page,'suspended native '+item.native_route);
 }finally{
  await more(page,'Close browser');
  await page.waitForFunction(()=>window.operations.some(o=>o.action==='close'&&o.running===false));
  closed=true;evidence.cleanup[item.session]={closed};save();await page.close();
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
 await mode(page,'Take control privately','private',false);
 await navigate(page,cfg.site+'/suspended-web-'+item.label);
 await decoded(page,'suspended Web '+item.label);
 await more(page,'Close browser');
 await page.waitForFunction(()=>window.mounted.session.status?.running===false&&!window.mounted.session.busy);
 evidence.cleanup[item.session]={closed:true};save();
 await page.evaluate(()=>window.mounted.dispose());await page.close();
}
try{
 browser=await chromium.launch({executablePath:cfg.chromium,headless:true,chromiumSandbox:true,args:['--disable-background-networking']});
 if(cfg.mode==='suspended-native'){for(const item of cfg.sessions)await suspendedNative(item);}
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
  await mode(page,'Take control privately','private');
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
  await mode(page,'Return to agent','agent');
  assert.equal(operations.filter(o=>o.action==='start').length,0,'no incidental host restart');
  assert.equal(operations.filter(o=>o.action==='attach').length,0,'reuse pre-attached fixture without duplicate attachment');
  await page.getByRole('button',{name:'Close viewer',exact:true}).click();
  await page.waitForFunction(()=>!document.querySelector('.browser-next-mirror'));
  await new Promise(r=>setTimeout(r,300));
  assert.equal((await p.op('status')).status.running,true,'close viewer must preserve host');
  evidence.steps.push({mounted:p.item.label,automatic_connections:1,history:true,modal:true,ime:true,close_viewer_preserved_host:true});save();
  await page.close();
 }
 await native(a.item,a);await native(b.item,b);
 }
 evidence.actions='passed';
}catch(e){evidence.failure={name:e.name,message:String(e.message).replace(/(?:file|https?):\/\/[^\s\"']+/g,'[private URL omitted]')};process.exitCode=1;}
finally{
 for(const p of peers){try{evidence.pre_teardown.push({session:p.item.session,...await p.op('status')});}catch(e){evidence.pre_teardown.push({error:String(e)});} }save();
 for(const p of peers){try{if(!p.binding||evidence.cleanup[p.item.session]?.closed){p.ws.close();continue;}const current=await p.op('status');if(current.status.running){if(!p.binding?.attachment_id||p.binding.attachment_id==='00000000-0000-0000-0000-000000000000'){p.binding={...current.status.binding,attachment_id:randomUUID()};await p.op('attach');}await p.op('close');}const s=await p.op('status');assert.equal(s.status.running,false);evidence.cleanup[p.item.session]=s;}catch(e){evidence.cleanup[p.item.session]={error:String(e)};process.exitCode=1;}p.ws.close();}
 if(browser)await browser.close();if(!process.exitCode&&evidence.actions==='passed')evidence.journey='passed';save();
}
// Ensure bounded completion even if a failed socket retained an unref'd timer.
setTimeout(()=>process.exit(process.exitCode||0),50);
