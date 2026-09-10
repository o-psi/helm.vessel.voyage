import { chromium } from 'playwright-core';
import fs from 'node:fs/promises';
import path from 'node:path';
import http from 'node:http';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { Refusal, refuse, token, digest, origin, networkProxy } from './security.mjs';

// stdout is the private parent protocol, never browser/process diagnostics.
process.umask(0o077);
const assets = path.dirname(fileURLToPath(import.meta.url));
const ID = /^[A-Za-z0-9_-]{1,128}$/;
const MAX_LINE = 3 * 1024 * 1024;
const S = { ready:false, closing:false, epoch:1, mode:'private', shared:false,
  browser_id:crypto.randomUUID(), pages:new Map(), refs:new Map(), observations:new Map(),
  receipts:new Map(), allowed:new Map(), queue:[], active:null, prompts:new Map(),
  uploads:new Map(), downloads:new Map(), controller:null, clients:new Map(), heartbeat:Date.now() };
let context, proxy, server, root, lock, companionOrigin, launchSecret = token(), watchdog, diskWatch, diskChecking=false, rasterBusy=false;
let limits = {max_tabs:8, max_receipts:10000, max_transfer_bytes:2*1024*1024, action_timeout_ms:15000,
  prompt_timeout_ms:30000, heartbeat_ms:5000, width:1280,height:720, max_profile_bytes:256*1024*1024};
const safeError = e => ({code:e instanceof Refusal ? e.code : 'operation_failed',message:e instanceof Refusal ? e.code : 'Local browser operation failed; details withheld'});
function status() { return {browser_id:S.browser_id,epoch:S.epoch,capture_epoch:S.epoch,mode:S.mode,shared:S.shared,ready:S.ready,
  pending:S.queue.length+(S.active?1:0),connected:Date.now()-S.heartbeat < limits.heartbeat_ms}; }
function fence(mode='private') {
  for(const v of S.refs.values())v.handle.dispose().catch(()=>{});
  S.boundRun=null;S.epoch++; S.mode=mode; S.shared=false; S.refs.clear(); S.observations.clear();
  for (const p of S.prompts.values()) p.resolve(false); S.prompts.clear();
  // Pending commands cannot inherit renewed authority. Dispatched effects remain unknown.
  for (const job of S.queue) job.cancelled=true;
  if (S.active) S.active.cancelled=true;
  if(S.ready)output({event:"control",...status()});
}
function authority(epoch) {
  if (!S.ready || S.closing || S.mode!=='agent' || !S.shared || epoch!==S.epoch
      || Date.now()-S.heartbeat>=limits.heartbeat_ms || (S.binding && Date.now()>=S.binding.expires_at_ms)) refuse('authority_fenced');
}
async function durable(receipt) {
  const file=path.join(root,'receipts',receipt.id+'.json'), temp=file+'.'+crypto.randomUUID()+'.tmp';
  const h=await fs.open(temp,'wx',0o600);
  try { await h.writeFile(JSON.stringify(receipt)); await h.sync(); } finally { await h.close(); }
  await fs.rename(temp,file);
  const d=await fs.open(path.dirname(file),'r'); try { await d.sync(); } finally { await d.close(); }
  S.receipts.set(receipt.id,receipt);
}
function pageFor(id) { const p=S.pages.get(id); if(!p || p.isClosed())refuse('stale_page'); return p; }
function pageId(p) { return [...S.pages].find(([,v])=>v===p)?.[0]; }
function activePage() { const p=S.pages.get(S.selected); if(p&&!p.isClosed())return p; refuse('no_page'); }
async function registerPage(p) {
  if(S.pages.size>=limits.max_tabs) { await p.close(); return; }
  const id=crypto.randomUUID(); S.pages.set(id,p); S.selected=id;
  p.setDefaultTimeout(limits.action_timeout_ms); p.setDefaultNavigationTimeout(limits.action_timeout_ms);
  p.on('close',()=> { S.pages.delete(id); invalidate(id); if(S.selected===id)S.selected=S.pages.keys().next().value; });
  p.on('framenavigated',()=>invalidate(id));
  p.on('dialog',dialog=> {
    // Never automatically accept a page's consequential dialog.
    if(S.dialog)S.dialog.dialog.dismiss().catch(()=>{});
    S.dialog={id:crypto.randomUUID(),page:id,type:dialog.type(),message:dialog.message().slice(0,2048),dialog};
    setTimeout(()=> { if(S.dialog?.dialog===dialog)S.dialog=null;dialog.dismiss().catch(()=>{}); },limits.prompt_timeout_ms).unref();
  });
  p.on('download',download=>stageDownload(download).catch(()=>{}));
  p.on('filechooser',chooser=> { S.chooser={id:crypto.randomUUID(),chooser,epoch:S.epoch}; });
}
function invalidate(id) { for(const [key,v]of S.observations)if(v.page===id)S.observations.delete(key); for(const[key,v]of S.refs)if(v.page===id){v.handle.dispose().catch(()=>{});S.refs.delete(key);} }
async function init(req) {
  if(S.ready||root)refuse('already_initialized');
  if(typeof req.session_dir!=='string'||!path.isAbsolute(req.session_dir))refuse('invalid_session_directory');
  root=req.session_dir;
  if(req.browser_id) {if(!ID.test(req.browser_id))refuse('invalid_browser_id');S.browser_id=req.browser_id;}
  S.label=typeof req.label==='string'?req.label.slice(0,300):'';S.binding=req.binding||null;
  await fs.mkdir(root,{recursive:true,mode:0o700});
  const st=await fs.lstat(root);
  if(!st.isDirectory()||st.isSymbolicLink()||(st.mode&0o077)||(process.getuid&&st.uid!==process.getuid())||await fs.realpath(root)!==path.resolve(root))refuse('unsafe_session_directory');
  lock=await fs.open(path.join(root,'executor.lock'),'wx',0o600).catch(()=>refuse('profile_locked'));
  await lock.writeFile(JSON.stringify({browser_id:S.browser_id,pid:process.pid})); await lock.sync();
  for(const name of ['receipts','downloads','uploads','profile']) {
    await fs.mkdir(path.join(root,name),{mode:0o700,recursive:true});
    const stat=await fs.lstat(path.join(root,name));if(!stat.isDirectory()||stat.isSymbolicLink()||(stat.mode&0o077)||(process.getuid&&stat.uid!==process.getuid()))refuse('unsafe_session_directory');
  }
  // Cleanly closed profiles may be reused. A crash lock requires explicit operator recovery.
  for(const name of await fs.readdir(path.join(root,'receipts'))) {
    if(!/^[A-Za-z0-9_-]{1,128}\.json$/.test(name))continue;
    const file=path.join(root,'receipts',name),st=await fs.lstat(file);if(!st.isFile()||st.isSymbolicLink()||st.size>4096||(st.mode&0o077))refuse('unsafe_receipt');
    const r=JSON.parse(await fs.readFile(file,'utf8'));if(!ID.test(r.id)||name!==r.id+'.json')refuse('invalid_receipt');
    if(r.state==='dispatched')r.state='unknown';if(r.state==='queued')r.state='cancelled_before_dispatch';await durable(r);
  }
  for(const [key,min,max]of [['max_tabs',1,16],['max_receipts',1,10000],['max_transfer_bytes',1024,2*1024*1024],['action_timeout_ms',1000,30000],['prompt_timeout_ms',1000,60000],['heartbeat_ms',2000,30000],['width',640,1600],['height',480,1000],['max_profile_bytes',32*1024*1024,1024*1024*1024]]) {
    const v=req.limits?.[key] ?? (key==='heartbeat_ms'?req.heartbeat_ms:undefined);
    if(v!==undefined) { if(!Number.isInteger(v)||v<min||v>max)refuse('invalid_limits'); limits[key]=v; }
  }
  await checkDisk();proxy=await networkProxy(S.allowed);
  const executable=req.executable_path || '/usr/bin/chromium';
  if(!path.isAbsolute(executable))refuse('invalid_executable');
  context=await chromium.launchPersistentContext(path.join(root,'profile'),{
    executablePath:executable,headless:true,chromiumSandbox:true,acceptDownloads:true,
    downloadsPath:path.join(root,'downloads'),serviceWorkers:'block',proxy:{server:proxy.server,username:proxy.username,password:proxy.password,bypass:'<-loopback>'},
    viewport:{width:limits.width,height:limits.height},deviceScaleFactor:1,
    args:['--disk-cache-size=16777216','--media-cache-size=16777216','--disable-quic','--force-webrtc-ip-handling-policy=disable_non_proxied_udp','--disable-background-networking','--disable-features=WebTransport'],
  });
  await context.route('**/*',async route=> {
    try { if(!S.allowed.has(origin(route.request().url())))refuse('origin_not_allowed'); await route.continue(); }
    catch { await route.abort('blockedbyclient').catch(()=>{}); }
  });
  await context.routeWebSocket('**/*',ws=>ws.close());
  context.on('page',p=>registerPage(p).catch(()=>{}));
  context.on('close',()=> { S.ready=false; fence(); });
  for(const p of context.pages())await registerPage(p);
  if(!S.pages.size)await context.newPage();
  await startCompanion(); S.ready=true; S.heartbeat=Date.now();
  watchdog=setInterval(()=> { if(S.shared && (Date.now()-S.heartbeat>=limits.heartbeat_ms || (S.binding && Date.now()>=S.binding.expires_at_ms)))fence(); if(S.controller&&Date.now()-S.controller.seen>5000){S.controller=null;fence();} },250); watchdog.unref();
  diskWatch=setInterval(()=>{if(!diskChecking){diskChecking=true;void checkDisk().catch(()=>{fence();void context?.close().catch(()=>{});}).finally(()=>diskChecking=false);}},1000);diskWatch.unref();
  return {...status(),companion_url:`${companionOrigin}/#${launchSecret}`,limits};
}
async function confirmation(job,summary) {
  authority(job.epoch);
  if(!S.controller||Date.now()-S.controller.seen>5000)refuse('local_controller_required');
  const id=crypto.randomUUID();
  output({event:'approval',request_id:job.id,pending:true});
  return await new Promise(resolve=> {
    const timer=setTimeout(()=>{S.prompts.delete(id);output({event:'approval',request_id:job.id,pending:false});resolve(false);},limits.prompt_timeout_ms);
    S.prompts.set(id,{id,request_id:job.id,summary,epoch:job.epoch,resolve:value=>{clearTimeout(timer);S.prompts.delete(id);output({event:'approval',request_id:job.id,pending:false});resolve(value);}});
  });
}
async function observe(p,epoch) {
  authority(epoch);
  const page=pageId(p); invalidate(page);
  const observation=crypto.randomUUID();
  const handles=await p.locator('a,button,input,textarea,select,[role="button"],[contenteditable="true"]').elementHandles();
  const refs=[];
  for(const h of handles.slice(0,200)) {
    authority(epoch);
    const data=await h.evaluate(e=>({tag:e.tagName.toLowerCase(),role:e.getAttribute('role'),type:e.getAttribute('type'),
      label:(e.getAttribute('aria-label')||e.textContent||e.getAttribute('placeholder')||'').slice(0,300),
      href:e.getAttribute('href'),disabled:!!e.disabled,password:e.tagName==='INPUT'&&e.type==='password',visible:!!(e.getBoundingClientRect().width&&e.getBoundingClientRect().height)}));
    if(!data.visible||data.password){await h.dispose();continue;}
    const ref=crypto.randomUUID();S.refs.set(ref,{handle:h,page,observation,epoch,signature:JSON.stringify(data)}); refs.push({ref,element:ref,...data});
  }
  for(const h of handles.slice(200))await h.dispose();
  const text=await p.locator('body').innerText({timeout:3000}).catch(()=> '');
  const title=await p.title(); authority(epoch);
  S.observations.set(observation,{page,epoch,created:Date.now()});
  return {page_id:page,observation_id:observation,url:p.url(),title,text:text.slice(0,16000),refs,epoch,
    uploads:[...S.uploads.values()].filter(u=>u.shared&&u.epoch===epoch).map(u=>({grant_id:u.id,size:u.size})),
    downloads:[...S.downloads.values()].filter(d=>d.disclose&&d.epoch===epoch&&!d.pending).map(d=>({download_id:d.id,size:d.size}))};
}
async function reference(a,epoch) {
  const v=S.refs.get(a.ref), o=S.observations.get(a.observation_id);
  if(!v||!o||v.page!==a.page_id||v.observation!==a.observation_id||v.epoch!==epoch||Date.now()-o.created>30000)refuse('stale_observation');
  const current=await v.handle.evaluate(e=>({tag:e.tagName.toLowerCase(),role:e.getAttribute('role'),type:e.getAttribute('type'),label:(e.getAttribute('aria-label')||e.textContent||e.getAttribute('placeholder')||'').slice(0,300),href:e.getAttribute('href'),disabled:!!e.disabled,password:e.tagName==='INPUT'&&e.type==='password',visible:!!(e.isConnected&&e.getBoundingClientRect().width&&e.getBoundingClientRect().height)}));
  authority(epoch);if(JSON.stringify(current)!==v.signature)refuse('stale_observation');
  return v.handle;
}
function fresh(a,epoch) {
  const o=S.observations.get(a.observation_id);
  if(!o||o.page!==a.page_id||o.epoch!==epoch||Date.now()-o.created>30000)refuse('stale_observation');
}
async function checkDisk() {
  let total=0,count=0;
  const walk=async dir=>{for(const item of await fs.readdir(dir,{withFileTypes:true})){if(++count>20000)refuse('profile_entry_limit');const file=path.join(dir,item.name);let st;try{st=await fs.lstat(file);}catch(e){if(e.code==='ENOENT')continue;throw e;}if(st.isSymbolicLink())continue;if(st.isDirectory())await walk(file);else if(st.isFile()){total+=st.size;if(total>limits.max_profile_bytes)refuse('profile_disk_limit');}}};
  await walk(root);return total;
}
async function raster(p) {
  if(rasterBusy)refuse('capture_busy');rasterBusy=true;
  try {
    const bytes=await p.screenshot({type:'jpeg',quality:65,fullPage:false,timeout:5000});
    if(bytes.length>2*1024*1024)refuse('screenshot_limit');
    return {mime_type:'image/jpeg',data_base64:bytes.toString('base64'),width:limits.width,height:limits.height};
  }finally{rasterBusy=false;}
}
function sameBinding(a,b) {
  return ['session_id','incarnation','browser_id','resource_id','executor_id'].every(k=>a[k]===b[k]);
}
function fields(value,keys) {
  if(!value||typeof value!=='object'||Array.isArray(value)||Object.keys(value).some(k=>!keys.includes(k)))refuse('invalid_action');
}
function normalizeAction(a) {
  if(!a||typeof a.action!=='string')refuse('invalid_action');
  const spec={inspect:['page_id'],navigate:['target','url'],click:['target','element'],fill:['target','element','text'],scroll:['target','delta_x','delta_y'],tabs:['operation'],screenshot:['target'],upload:['target','element','grant_id'],download:['target','download_id'],upload_prepare:['transfer_id','name','mime_type','data_base64']}[a.action];
  if(!spec)refuse('unsupported_action');fields(a,['action',...spec]);
  let n={...a,type:a.action};
  if(a.target){fields(a.target,['page_id','observation_id']);if(!ID.test(a.target.page_id)||!ID.test(a.target.observation_id))refuse('invalid_target');Object.assign(n,a.target);}
  if(spec.includes('target')&&!a.target)refuse('invalid_target');
  if(a.action==='inspect')n.type='observe';
  if(a.action==='upload_prepare')n.type='upload_stage';
  if(a.element!==undefined)n.ref=a.element;
  if(a.grant_id!==undefined)n.upload_id=a.grant_id;
  if(a.action==='scroll'){n.x=a.delta_x;n.y=a.delta_y;}
  if(a.action==='tabs') {
    const keys={list:[],open:['url'],select:['target'],close:['target']}[a.operation?.operation];
    if(!keys)refuse('invalid_tabs');fields(a.operation,['operation',...keys]);
    n.type={list:'tabs',open:'tab_open',select:'tab_select',close:'tab_close'}[a.operation.operation];
    n.url=a.operation.url;
    if(keys.includes('target')){fields(a.operation.target,['page_id','observation_id']);Object.assign(n,a.operation.target);}
  }
  return n;
}
function wireResult(job,rec,result,error) {
  const state=rec.state==='completed'?'completed':rec.state==='unknown'||rec.state==='dispatched'?'unresolved':rec.state==='cancelled_before_dispatch'?'cancelled':'refused';
  const image=result?.mime_type==='image/jpeg'?{mime_type:result.mime_type,data_base64:result.data_base64}:null;
  const file=result?.download_id&&result?.data_base64?{name:'download.bin',mime_type:'application/octet-stream',data_base64:result.data_base64}:null;
  const text=error?error.code:image?'Browser screenshot':file?'Locally approved download':JSON.stringify(result??{receipt:rec,content_withheld:true});
  return {request_id:job.id,action_sha256:job.action_sha256,state,text,page_id:result?.page_id||null,observation_id:result?.observation_id||null,image,file};
}
async function dispatch(job,fn) {
  authority(job.epoch);if(job.cancelled)refuse('cancelled_before_dispatch');
  if(job.expires_at_ms&&job.expires_at_ms<=Date.now())refuse('request_expired');
  const rec=S.receipts.get(job.id);rec.state='dispatched';await durable(rec);
  authority(job.epoch);if(job.cancelled)refuse('cancelled_before_dispatch');
  return fn();
}
async function execute(job) {
  const a=normalizeAction(job.action);
  if(!a||typeof a!=='object')refuse('invalid_action');
  authority(job.epoch);if(job.expires_at_ms&&job.expires_at_ms<=Date.now())refuse('request_expired');
  const op=a.type || a.op;
  const reads=['observe','tabs','screenshot'];
  if(!reads.includes(op)) {
    if(!await confirmation(job,{type:op,page_id:a.page_id || null,url:op==='navigate'?String(a.url).slice(0,2048):undefined,ref:a.ref || null}))refuse('local_confirmation_denied');
    authority(job.epoch);if(job.expires_at_ms&&job.expires_at_ms<=Date.now())refuse('request_expired'); if(job.cancelled)refuse('cancelled_before_dispatch');
  }
  switch(op) {
    case 'tabs': return {tabs:await Promise.all([...S.pages].map(async([id,p])=>({page_id:id,url:p.url(),title:await p.title()})))};
    case 'observe': return observe(a.page_id?pageFor(a.page_id):activePage(),job.epoch);
    case 'navigate': {
      const u=String(a.url); if(u.length>8192||!S.allowed.has(origin(u)))refuse('origin_not_allowed');
      fresh(a,job.epoch);const p=pageFor(a.page_id);
      await dispatch(job,()=>p.goto(u,{waitUntil:'domcontentloaded'})); return observe(p,job.epoch);
    }
    case 'click': { const h=await reference(a,job.epoch);authority(job.epoch);await dispatch(job,()=>h.click({force:true,timeout:1000})); invalidate(a.page_id);return {effect:'completed',requires_observation:true}; }
    case 'fill': { if(typeof a.text!=='string'||a.text.length>10000)refuse('invalid_text');const h=await reference(a,job.epoch);
      if(await h.evaluate(e=>e.type==='password'))refuse('private_input_required');
      authority(job.epoch);await dispatch(job,()=>h.fill(a.text,{force:true,timeout:1000})); invalidate(a.page_id);return {effect:'completed',requires_observation:true}; }
    case 'scroll': { fresh(a,job.epoch);const p=pageFor(a.page_id);const x=Number(a.x||0),y=Number(a.y||0);if(!Number.isFinite(x)||!Number.isFinite(y)||Math.abs(x)>5000||Math.abs(y)>5000)refuse('invalid_scroll');await dispatch(job,()=>p.mouse.wheel(x,y));invalidate(a.page_id);return {effect:'completed',requires_observation:true}; }
    case 'screenshot': {fresh(a,job.epoch);return {...await raster(pageFor(a.page_id)),page_id:a.page_id,observation_id:a.observation_id,epoch:job.epoch};}
    case 'tab_open': {if(S.pages.size>=limits.max_tabs)refuse('tab_limit');if(!S.allowed.has(origin(a.url)))refuse('origin_not_allowed');const p=await dispatch(job,()=>context.newPage());authority(job.epoch);await p.goto(a.url,{waitUntil:'domcontentloaded'});return observe(p,job.epoch); }
    case 'tab_select': fresh(a,job.epoch);pageFor(a.page_id);await dispatch(job,()=>{S.selected=a.page_id;});return observe(pageFor(a.page_id),job.epoch);
    case 'tab_close': fresh(a,job.epoch);await dispatch(job,()=>pageFor(a.page_id).close());return {closed:true};
    case 'upload': { const h=await reference(a,job.epoch),grant=S.uploads.get(a.upload_id);if(!grant||!grant.shared||grant.epoch!==job.epoch)refuse('upload_not_granted');authority(job.epoch);await dispatch(job,()=>h.setInputFiles(grant.file,{timeout:1000}));S.uploads.delete(a.upload_id);await fs.unlink(grant.file);invalidate(a.page_id);return {effect:'completed',requires_observation:true}; }
    case 'download': { fresh(a,job.epoch);const d=S.downloads.get(a.download_id);if(!d||!d.disclose||d.epoch!==job.epoch)refuse('download_not_granted');const b=await fs.readFile(d.file);if(b.length>limits.max_transfer_bytes)refuse('transfer_limit');authority(job.epoch);d.disclose=false;return {download_id:a.download_id,mime_type:'application/octet-stream',data_base64:b.toString('base64')}; }
    case 'upload_stage': { if(typeof a.data_base64!=='string'||a.data_base64.length>limits.max_transfer_bytes*1.4||! /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(a.data_base64))refuse('invalid_blob');const b=Buffer.from(a.data_base64,'base64');if(b.length>limits.max_transfer_bytes)refuse('transfer_limit');return dispatch(job,()=>stageUpload(b,'remote blob',false)); }
    default: refuse('unsupported_action');
  }
}
async function drain() {
  if(S.active)return;
  const job=S.queue.shift();if(!job)return;
  S.active=job;
  let result,error;
  let deadline;
  const expired=new Promise((_,reject)=> {deadline=setTimeout(()=>{fence();void context?.close().catch(()=>{});reject(new Refusal('action_deadline_unknown'));},limits.action_timeout_ms+limits.prompt_timeout_ms+2000);});
  try { if(job.cancelled)refuse('cancelled_before_dispatch');authority(job.epoch);result=await Promise.race([execute(job),expired]);authority(job.epoch);if(job.cancelled)refuse('authority_fenced'); }
  catch(e) {error=safeError(e);}
  finally{clearTimeout(deadline);}
  const rec=S.receipts.get(job.id);
  rec.state=error?((job.cancelled&&rec.state!=='dispatched')||error.code==='cancelled_before_dispatch'?'cancelled_before_dispatch':rec.state==='dispatched'?'unknown':'refused'):'completed';
  rec.code=error?.code || null;
  try { await durable(rec); } catch { fence();error={code:'receipt_persistence_failed',message:'Outcome unknown'}; }
  // No DOM, screenshot, dialog or private input survives an authority transition.
  if(job.epoch!==S.epoch || job.cancelled) { result=undefined;error={code:rec.state==='cancelled_before_dispatch'?'cancelled_before_dispatch':'unknown',message:'Authority changed; content withheld'}; }
  job.resolve({id:job.id,ok:true,result:wireResult(job,rec,result,error)});
  S.active=null;void drain();
}
async function action(req) {
  if(typeof req.id!=='string'||!ID.test(req.id))refuse('invalid_id');
  const hash=digest(req),prev=S.receipts.get(req.id);
  if(prev) { if(prev.digest!==hash)refuse('request_id_conflict');return {id:req.id,ok:true,result:wireResult(req,prev,null,{code:'duplicate_content_withheld'})}; }
  authority(req.epoch);
  if(req.capture_epoch!==S.epoch)refuse('capture_fenced');
  if(S.binding&&!req.binding)refuse('binding_required');
  if(req.binding && (!S.binding || !sameBinding(req.binding,S.binding) || req.binding.controller_epoch!==S.epoch || req.binding.capture_epoch!==S.epoch || req.binding.expires_at_ms<=Date.now()))refuse('binding_fenced');
  if(req.binding){if(!req.binding.run_id)refuse('run_required');if(S.boundRun&&S.boundRun!==req.binding.run_id){fence();refuse('run_changed');}S.boundRun=req.binding.run_id;}
  if(typeof req.action_sha256!=='string'||! /^[a-f0-9]{64}$/.test(req.action_sha256))refuse('invalid_action_digest');
  normalizeAction(req.action);
  if(S.queue.length>=16||S.receipts.size>=limits.max_receipts)refuse('resource_limit');
  const rec={id:req.id,digest:hash,action_sha256:req.action_sha256,state:'queued',epoch:req.epoch,created_at:Date.now()};
  // Reserve before the first await, preventing concurrent duplicate admissions.
  S.receipts.set(req.id,rec);await durable(rec);
  return new Promise(resolve=> { S.queue.push({...req,resolve,cancelled:false});void drain(); });
}
async function stageUpload(bytes,name,shared) {
  if(S.uploads.size>=8||bytes.length>limits.max_transfer_bytes)refuse('transfer_limit');
  const id=crypto.randomUUID(),file=path.join(root,'uploads',id);
  await fs.writeFile(file,bytes,{mode:0o600,flag:'wx'});
  S.uploads.set(id,{id,file,name:String(name).slice(0,160),size:bytes.length,shared,epoch:S.epoch});
  return {upload_id:id,grant_id:id,size:bytes.length};
}
async function stageDownload(download) {
  const id=crypto.randomUUID();
  if(S.downloads.size>=8) {await download.cancel();return;}
  // Chromium staging is watched while receiving; cancelled files are removed.
  const entry={id,name:download.suggestedFilename().slice(0,160),size:0,disclose:false,epoch:S.epoch,pending:true};S.downloads.set(id,entry);
  let exceeded=false;
  const timer=setInterval(async()=> {try {const names=await fs.readdir(path.join(root,'downloads'));let total=0;for(const n of names)total+=(await fs.stat(path.join(root,'downloads',n))).size;if(total>limits.max_transfer_bytes*8){exceeded=true;await download.cancel();}}catch{}},100);
  const timeout=setTimeout(()=>download.cancel().catch(()=>{}),30000);
  try {
    const file=await download.path();if(exceeded||!file)refuse('transfer_limit');
    const st=await fs.stat(file);if(st.size>limits.max_transfer_bytes)refuse('transfer_limit');
    entry.file=file;entry.size=st.size;entry.pending=false;
  }catch{S.downloads.delete(id);await download.delete().catch(()=>{});}
  finally{clearInterval(timer);clearTimeout(timeout);}
}
async function close() {
  if(S.closing)return; S.closing=true; fence();clearInterval(watchdog);clearInterval(diskWatch);
  if(context)await context.close().catch(()=>{});
  if(proxy)await proxy.close();
  if(server){server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
  // Never remove profile or receipts. Staged private files are session-owned and removed on clean shutdown.
  if(root&&lock) { for(const name of ['uploads','downloads'])await fs.rm(path.join(root,name),{recursive:true,force:true});await lock.close();await fs.unlink(path.join(root,'executor.lock')).catch(()=>{}); }
  S.ready=false;
}
async function request(req) {
  if(!req||typeof req.id!=='string'||!ID.test(req.id)||typeof req.op!=='string')refuse('invalid_request');
  if(req.op==='init')return init(req);
  if(req.op==='shutdown') {await close();return {closed:true};}
  if(!S.ready)refuse('not_initialized');
  if(req.op==='heartbeat'){if(req.binding){if((S.binding&&!sameBinding(S.binding,req.binding))||(req.binding.run_id&&S.boundRun&&req.binding.run_id!==S.boundRun))fence();S.binding=req.binding;}S.heartbeat=Date.now();return status();}
  if(req.op==='status')return status();
  if(req.op==='control'){if(!['private','human'].includes(req.mode))refuse('local_sharing_required');fence(req.mode);return status();}
  if(req.op==='receipt'){return {receipt:S.receipts.get(req.request_id)||null};}
  if(req.op==='cancel'){
    const job=[S.active,...S.queue].find(j=>j?.id===req.request_id);
    if(job){job.cancelled=true;if(job===S.active)fence();}
    return {receipt:S.receipts.get(req.request_id)||null,cancellation_requested:!!job};
  }
  refuse('unsupported_operation');
}
async function body(req,max=65536) {
  let size=0;const chunks=[];
  for await(const b of req){size+=b.length;if(size>max)refuse('request_limit');chunks.push(b);}
  try{return JSON.parse(Buffer.concat(chunks).toString('utf8'));}catch{refuse('invalid_json');}
}
function localClient(req) {
  const cookie=req.headers.cookie?.split(';').map(s=>s.trim()).find(s=>s.startsWith('helm_browser='))?.slice(13);
  const c=S.clients.get(cookie);
  if(!c||req.headers['x-helm-csrf']!==c.csrf)refuse('local_auth_required');
  return c;
}
function controller(c,b) {
  if(!S.controller||S.controller.client!==c||S.controller.id!==b.controller_id)refuse('not_controller');
  S.controller.seen=Date.now();
}
function human(c,b) {controller(c,b);if(S.active)refuse('agent_effect_settling');if(!['human','private'].includes(S.mode)||b.epoch!==S.epoch)refuse('human_control_required');}
async function localOperation(c,b) {
  if(b.op==='claim') {
    if(S.controller&&Date.now()-S.controller.seen<5000&&S.controller.id!==b.controller_id)refuse('controller_busy');
    if(typeof b.controller_id!=='string'||!ID.test(b.controller_id))refuse('invalid_controller');
    S.controller={client:c,id:b.controller_id,seen:Date.now()};return status();
  }
  controller(c,b);
  if(b.op==='state')return {...status(),width:limits.width,height:limits.height,label:S.label,selected:S.selected,
    tabs:await Promise.all([...S.pages].map(async([id,p])=>({id,url:p.url(),title:await p.title().catch(()=> '')}))),
    origins:[...S.allowed].map(([url,v])=>({url,...v})),prompts:[...S.prompts.values()].map(({id,summary})=>({id,summary})),
    uploads:[...S.uploads.values()].map(({id,name,size,shared})=>({id,name,size,shared})),downloads:[...S.downloads.values()].map(({id,name,size,pending,disclose})=>({id,name,size,pending,disclose})),
    dialog:S.dialog?{id:S.dialog.id,type:S.dialog.type,message:S.dialog.message}:null,chooser:S.chooser?.id};
  if(b.op==='mode') {
    if(!['agent','human','private'].includes(b.mode))refuse('invalid_mode');
    if(b.mode==='agent'&&S.localBusy)refuse('local_effect_settling');
    fence(b.mode);
    if(b.mode==='agent'){if(Date.now()-S.heartbeat>=limits.heartbeat_ms||!b.confirm_share) {fence();refuse('sharing_not_confirmed');}S.shared=true;output({event:'control',...status()});}
    return status();
  }
  if(b.op==='confirm') {const p=S.prompts.get(b.prompt_id);if(!p||p.epoch!==S.epoch)refuse('stale_prompt');p.resolve(b.allow===true);return {};}
  if(b.op==='origin') {
    human(c,b);const o=origin(b.url);if(b.remove){S.allowed.delete(o);proxy.fence();}else{if(S.allowed.size>=64&&!S.allowed.has(o))refuse('origin_limit');S.allowed.set(o,{private_network:b.private_network===true});}
    return {};
  }
  if(b.op==='share_upload') {const u=S.uploads.get(b.upload_id);if(!u)refuse('missing_upload');u.shared=b.allow===true;u.epoch=S.epoch;return {};}
  if(b.op==='disclose_download') {const d=S.downloads.get(b.download_id);if(!d||d.pending)refuse('missing_download');d.disclose=b.allow===true;d.epoch=S.epoch;return {};}
  human(c,b);
  const p=activePage();
  switch(b.op) {
    case 'navigate': if(!S.allowed.has(origin(b.url)))refuse('origin_not_allowed');await p.goto(b.url,{waitUntil:'domcontentloaded'});break;
    case 'tab':pageFor(b.page_id);S.selected=b.page_id;break;
    case 'new_tab':await context.newPage();break;
    case 'close_tab':await p.close();break;
    case 'pointer': {
      if(!Number.isFinite(b.x)||!Number.isFinite(b.y)||b.x<0||b.y<0||b.x>limits.width||b.y>limits.height)refuse('invalid_coordinates');
      if(b.kind==='move')await p.mouse.move(b.x,b.y);
      else if(b.kind==='down'||b.kind==='up'){await p.mouse.move(b.x,b.y);await p.mouse[b.kind]({button:['left','middle','right'][b.button]||'left',clickCount:b.click_count===2?2:1});}
      else refuse('invalid_pointer');break;
    }
    case 'wheel':if(!Number.isFinite(b.x)||!Number.isFinite(b.y)||Math.abs(b.x)>5000||Math.abs(b.y)>5000)refuse('invalid_scroll');await p.mouse.wheel(b.x,b.y);break;
    case 'key':if(typeof b.key!=='string'||b.key.length>40||!['down','up'].includes(b.kind))refuse('invalid_key');await p.keyboard[b.kind](b.key);break;
    case 'text':if(typeof b.text!=='string'||b.text.length>10000)refuse('invalid_text');await p.keyboard.insertText(b.text);break;
    case 'clipboard':if(b.allow!==true)refuse('clipboard_permission_required');if(typeof b.text!=='string'||b.text.length>10000)refuse('invalid_text');await p.keyboard.insertText(b.text);break;
    case 'upload': {if(typeof b.data_base64!=='string'||b.data_base64.length>limits.max_transfer_bytes*1.4)refuse('transfer_limit');const result=await stageUpload(Buffer.from(b.data_base64,'base64'),b.name,false);
      if(b.chooser_id){const ch=S.chooser;if(!ch||ch.id!==b.chooser_id||ch.epoch!==S.epoch)refuse('stale_chooser');const u=S.uploads.get(result.upload_id);await ch.chooser.setFiles(u.file);S.chooser=null;}return result;}
    case 'dialog':if(!S.dialog||S.dialog.id!==b.dialog_id)refuse('stale_dialog');if(b.accept)await S.dialog.dialog.accept(typeof b.text==='string'?b.text.slice(0,4096):'');else await S.dialog.dialog.dismiss();S.dialog=null;break;
    default:refuse('unsupported_local_operation');
  }
  return {};
}
async function startCompanion() {
  const securityHeaders={'Cache-Control':'no-store','Content-Security-Policy':"default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",'X-Content-Type-Options':'nosniff','Referrer-Policy':'no-referrer','Cross-Origin-Resource-Policy':'same-origin','Cross-Origin-Opener-Policy':'same-origin','Permissions-Policy':'camera=(), microphone=(), geolocation=()'};
  server=http.createServer(async(req,res)=> {
    for(const[k,v]of Object.entries(securityHeaders))res.setHeader(k,v);
    const reply=(code,obj)=>{if(res.writableEnded)return;res.writeHead(code,{'Content-Type':'application/json'});res.end(JSON.stringify(obj));};
    try {
      if(req.headers.host!==new URL(companionOrigin).host||!['127.0.0.1','::ffff:127.0.0.1'].includes(req.socket.remoteAddress)||req.url.includes('?'))refuse('invalid_host');
      if(req.method==='GET'&&['/','/app.js','/app.css'].includes(req.url)) {
        const name=req.url==='/'?'index.html':req.url.slice(1),type={'index.html':'text/html; charset=utf-8','app.js':'text/javascript; charset=utf-8','app.css':'text/css; charset=utf-8'}[name];
        res.writeHead(200,{'Content-Type':type});res.end(await fs.readFile(path.join(assets,name)));return;
      }
      if(req.headers.origin!==companionOrigin || req.method!=='POST')refuse('invalid_origin');
      if(req.headers['content-type']!=='application/json')refuse('invalid_content_type');
      const b=await body(req,MAX_LINE);
      if(req.url==='/session') {
        const previous=req.headers.cookie?.split(';').map(s=>s.trim()).find(s=>s.startsWith('helm_browser='))?.slice(13);
        if(!launchSecret&&S.clients.has(previous)){reply(200,{csrf:S.clients.get(previous).csrf});return;}
        if(typeof b.secret!=='string'||!launchSecret||b.secret.length!==launchSecret.length||!crypto.timingSafeEqual(Buffer.from(b.secret),Buffer.from(launchSecret)))refuse('local_auth_required');
        const cookie=token(),csrf=token();S.clients.set(cookie,{csrf});launchSecret=null;
        res.setHeader('Set-Cookie',`helm_browser=${cookie}; HttpOnly; SameSite=Strict; Path=/`);reply(200,{csrf});return;
      }
      // Page reload can recover CSRF only with strict same-origin JSON POST and the HttpOnly session cookie.
      if(req.url==='/resume') {const cookie=req.headers.cookie?.split(';').map(s=>s.trim()).find(s=>s.startsWith('helm_browser='))?.slice(13);const c=S.clients.get(cookie);if(!c)refuse('local_auth_required');reply(200,{csrf:c.csrf});return;}
      const c=localClient(req);
      if(req.url==='/frame') {controller(c,b);if(c.frameBusy||Date.now()-(c.lastFrame||0)<100)refuse('frame_rate_limit');c.frameBusy=true;c.lastFrame=Date.now();try{const epoch=S.epoch,p=activePage();const image=await raster(p);if(epoch!==S.epoch)refuse('stale_frame');res.writeHead(200,{'Content-Type':'image/jpeg'});res.end(Buffer.from(image.data_base64,'base64'));return;}finally{c.frameBusy=false;}}
      if(req.url==='/save') {controller(c,b);const d=S.downloads.get(b.download_id);if(!d||d.pending)refuse('missing_download');res.writeHead(200,{'Content-Type':'application/octet-stream','Content-Disposition':'attachment; filename="download.bin"'});res.end(await fs.readFile(d.file));return;}
      if(req.url!=='/api')refuse('not_found');
      const independent=['state','claim','mode','confirm'].includes(b.op);
      if(!independent&&S.localBusy)refuse('local_input_busy');
      if(!independent)S.localBusy=true;
      try{reply(200,await localOperation(c,b));}finally{if(!independent)S.localBusy=false;}
    }catch(e){reply(403,{error:safeError(e)});}
  });
  server.requestTimeout=10000;server.headersTimeout=5000;server.maxConnections=16;
  server.on('clientError',(_e,s)=>s.destroy());
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));companionOrigin=`http://127.0.0.1:${server.address().port}`;
}
let input=Buffer.alloc(0),fatal=false;
function output(obj){if(!fatal)process.stdout.write(JSON.stringify(obj)+'\n',()=>{if(S.closing&&obj?.ok&&obj?.result?.closed)process.exit(0);});}
process.stdin.on('data',chunk=> {
  input=Buffer.concat([input,chunk]);
  if(input.length>MAX_LINE&&!input.includes(10)){fatal=true;void close().finally(()=>process.exit(1));return;}
  let idx;
  while((idx=input.indexOf(10))!==-1){const line=input.subarray(0,idx);input=input.subarray(idx+1);if(line.length>MAX_LINE){fatal=true;void close().finally(()=>process.exit(1));return;}
    let req;try{req=JSON.parse(line);}catch{output({id:null,ok:false,error:{code:'invalid_json',message:'Invalid JSON'}});continue;}
    const id=typeof req?.id==='string'&&ID.test(req.id)?req.id:null;
    void (req?.op==='action'?action(req):request(req).then(result=>({id,ok:true,result})))
      .then(output).catch(e=>output({id,ok:false,error:safeError(e)}));
  }
});
process.stdin.on('end',()=>{fence();void close().finally(()=>process.exit(0));});
process.stdout.on('error',()=>{fatal=true;fence();void close().finally(()=>process.exit(1));});
for(const sig of ['SIGTERM','SIGINT'])process.on(sig,()=>{fence();void close().finally(()=>process.exit(0));});
process.on('uncaughtException',()=>{fatal=true;fence();process.stderr.write('Local browser helper stopped after an internal failure.\n');void close().finally(()=>process.exit(1));});
process.on('unhandledRejection',()=>{fatal=true;fence();process.stderr.write('Local browser helper stopped after an internal failure.\n');void close().finally(()=>process.exit(1));});
