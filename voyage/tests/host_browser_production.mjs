import {idleWindowQualified,idleReconnectQualified} from './host_browser_idle.mjs';
// Actual deployed React + TUI-launcher qualification. No socket/route rewrites,
// server authentication bypass, ambient browser access or inference submission.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {constants} from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash,randomUUID} from 'node:crypto';
import {execFile} from 'node:child_process';
import {performance} from 'node:perf_hooks';
import {browserCost,inputVisibleLatency} from './host_browser_client_cost.mjs';
import {requestNativeReopen,nativePageOwner} from './host_browser_native_reopen.mjs';
import {measurementWindowAligned,privateBindingSnapshot,privateReclaimQualified} from './host_browser_qualification_windows.mjs';
import {mediaColor,mediaMovementFacts} from './host_browser_media_pixels.mjs';

const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const uuid=value=>typeof value==='string'&&/^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/.test(value);
const label=value=>typeof value==='string'&&/^[A-Za-z0-9_.-]{1,40}$/.test(value);
const hash=value=>createHash('sha256').update(value).digest('hex');
const effect=action=>!['status','mirror','receipt'].includes(action);
const tracked=new WeakMap();
const https=value=>{const url=new URL(value);assert.equal(url.protocol,'https:');assert.ok(!url.username&&!url.password&&!url.hash);return url;};
const nativeMode=cfg=>cfg.native_route?.mode||'public';
const nativeScope=cfg=>nativeMode(cfg)==='local'?'owned native Helm local WS application bytes':'owned native Helm public WSS application bytes';
const nativeCounterScope=cfg=>nativeMode(cfg)==='local'?'native local WS Text application bytes; excludes HTTP upgrade, TCP and control frames':'native WSS Text application bytes; excludes HTTP upgrade, TLS/TCP and control frames';
function nativeWireSummary(wire,cfg){
 assert.equal(wire.schema,1);assert.equal(wire.scope,nativeScope(cfg));assert.equal(wire.verified_identity,true);
 assert.equal(wire.roots.length,2);
 for(let i=0;i<2;i++)for(const key of ['label','pid','start_ticks'])assert.equal(wire.roots[i][key],cfg.client_roots[i][key]);
 assert.equal(wire.windows.length,9);const conditions=['a_one_viewer','a_two_viewers','ab_four_viewers'];
 return {scope:wire.scope,verified_identity:true,windows:wire.windows.map((w,i)=>{
  assert.equal(w.condition,conditions[Math.floor(i/3)]);assert.equal(w.index,i%3);
  assert.ok(Number.isSafeInteger(w.sent_bytes)&&w.sent_bytes>=0&&Number.isSafeInteger(w.received_bytes)&&w.received_bytes>0);
  return {condition:w.condition,index:w.index,sent_bytes:w.sent_bytes,received_bytes:w.received_bytes};
 })};
}
async function localAuthorityProof(cfg,items,prior=null){
 if(nativeMode(cfg)!=='local')return null;
 const native=await nativeSnapshot(cfg);
 const sessions=items.map((item,i)=>({label:cfg.sessions[i].label,session_id:cfg.sessions[i].id,
  socket_id:native[i].socket_id,root:cfg.sessions[i].host_browser_root,claims:[...item.state.native_claims.values()]}));
 assert.ok(sessions.every(s=>s.claims.length>0&&s.claims.length<=32),'completed native noninput receipt required');
 const h=cfg.host_observer;const reply=JSON.parse(await sshCommand(cfg,[h.python,'-I',h.probe_script,'local-authority'],
  {schema:1,directory:cfg.native_route.directory,vessel_id:cfg.native_route.expected_vessel_id,sessions}));
 assert.equal(reply.mode,'local');assert.equal(reply.no_effects,true);assert.equal(reply.identities.length,2);assert.ok(/^[a-f0-9]{64}$/.test(reply.physical_vessel_identity_sha256));
 if(prior)assert.equal(reply.physical_vessel_identity_sha256,prior.physical_vessel_identity_sha256);
 for(let i=0;i<2;i++){
  const row=reply.identities[i];for(const key of ['label','session_id','socket_id'])assert.equal(row[key],sessions[i][key]);
  assert.equal(row.vessel_id,cfg.native_route.expected_vessel_id);assert.ok(uuid(row.principal_id)&&uuid(row.installation_id));
  assert.deepEqual(row.completed_native_receipt_ids,sessions[i].claims.map(c=>c.command_id));
  if(prior)for(const key of ['session_id','principal_id','installation_id','actor_sha256','vessel_id','socket_id'])assert.equal(row[key],prior.identities[i][key]);
 }
 return reply;
}
async function nativeSnapshot(cfg){
 const values=[];
 for(let i=0;i<2;i++){
  let value;const deadline=Date.now()+1000;
  // The passive native writer uses a pinned inode. Repeating this bounded
  // metadata read across its 200ms write is safe; no command/effect is sent.
  while(!value){try{value=await privateJson(cfg.native_metrics_files[i],4096,true);}catch{
   if(Date.now()>=deadline)throw Error('native byte observation unavailable');await sleep(25);
  }}
  cfg.native_metrics_identity??=[];const pinned=cfg.native_metrics_identity[i]??=value.identity;
  assert.deepEqual(value.identity,pinned);value={...value.value,inode:pinned};
  const root=cfg.client_roots[i];for(const key of ['label','pid','start_ticks'])assert.equal(value[key],root[key]);
  assert.equal(value.scope,nativeCounterScope(cfg));assert.equal(value.transport,nativeMode(cfg)==='local'?'local_ws':'public_wss');
  assert.ok(uuid(value.vessel_id)&&uuid(value.socket_id));
  if(nativeMode(cfg)==='local'){assert.equal(value.vessel_id,cfg.native_route.expected_vessel_id);assert.equal(value.authority,'existing local account owner');}
  assert.equal(value.status,'observed');assert.equal(value.routes,1);assert.ok(value.connections>=1);assert.equal(value.transport_failures,0);
  assert.equal(value.active,1);assert.equal(value.handshake_failures,0);
  assert.ok(Date.now()-value.captured_at_ms>=0&&Date.now()-value.captured_at_ms<=1000);
  for(const key of ['sent_bytes','received_bytes','sent_frames','received_frames','elapsed_ms','captured_at_ms','connections','attempts','disconnected','transport_failures','handshake_failures'])assert.ok(Number.isSafeInteger(value[key])&&value[key]>=0);
  values.push(value);
 }
 return values;
}
function nativeWindow(condition,index,before,after){
 const selected=condition==='ab_four_viewers'?[0,1]:[0];
 const clients=selected.map(i=>{
  assert.ok(after[i].captured_at_ms>before[i].captured_at_ms);
  assert.deepEqual(after[i].inode,before[i].inode);
  assert.equal(after[i].vessel_id,before[i].vessel_id);assert.equal(after[i].socket_id,before[i].socket_id);
  for(const key of ['attempts','connections','disconnected','transport_failures'])assert.equal(after[i][key],before[i][key]);
  for(const sample of [before[i],after[i]])assert.ok(Number.isFinite(sample.source_age_ms)&&sample.source_age_ms<=2000);
  for(const key of ['sent_bytes','received_bytes'])assert.ok(after[i][key]>=before[i][key]);
  return {label:before[i].label,vessel_id:before[i].vessel_id,socket_id:before[i].socket_id,authority:before[i].authority,started_at_ms:before[i].captured_at_ms,ended_at_ms:after[i].captured_at_ms,
   sent_bytes:after[i].sent_bytes-before[i].sent_bytes,received_bytes:after[i].received_bytes-before[i].received_bytes};
 });
 return {condition,index,sent_bytes:clients.reduce((n,c)=>n+c.sent_bytes,0),received_bytes:clients.reduce((n,c)=>n+c.received_bytes,0),clients};
}
async function saveNativeWire(cfg,windows){
 const wire={schema:1,scope:nativeScope(cfg),transport:nativeMode(cfg),verified_identity:true,
  roots:cfg.client_roots.map(({label,pid,start_ticks})=>({label,pid,start_ticks})),windows};
 await save(cfg.native_wire_evidence,wire);return nativeWireSummary(wire,cfg);
}
async function privateJson(file,limit=1024*1024,withIdentity=false){
 const handle=await fs.open(file,constants.O_RDONLY|constants.O_NOFOLLOW|constants.O_NONBLOCK|constants.O_CLOEXEC);
 try{const meta=await handle.stat();assert.ok(meta.isFile()&&meta.uid===process.getuid()&&meta.nlink===1&&!(meta.mode&0o077)&&meta.size<=limit);
 const bytes=Buffer.alloc(meta.size+1),{bytesRead}=await handle.read(bytes,0,bytes.length,0),after=await handle.stat();
  assert.equal(bytesRead,meta.size);for(const key of ['dev','ino','size','mtimeMs','ctimeMs'])assert.equal(meta[key],after[key]);
  assert.ok(after.uid===process.getuid()&&after.nlink===1&&!(after.mode&0o077));
  const value=JSON.parse(bytes.subarray(0,bytesRead));return withIdentity?{value,identity:{dev:meta.dev,ino:meta.ino}}:value;
 }finally{await handle.close();}
}
async function save(file,value){await fs.writeFile(file,JSON.stringify(value,null,2),{mode:0o600,flag:'wx'});}
async function until(predicate,ms=30000){const deadline=performance.now()+ms;while(!await predicate()){assert.ok(performance.now()<deadline,'bounded observation expired');await sleep(50);}}
function failureLocations(error){
 // Retain only coordinates in this reviewed source, never error messages,
 // assertion operands, URLs, launcher paths or other stack-frame contents.
 const source=import.meta.url.replace(/[.*+?^${}()|[\]\\]/g,'\\$&');
 const frame=new RegExp('^\\s+at (?:[^\\n]*\\()?'+source+':([0-9]+):([0-9]+)\\)?$');
 return typeof error?.stack==='string'?error.stack.split('\n').slice(1).flatMap(line=>{
  const match=frame.exec(line);return match?[{source:'host_browser_production.mjs',line:Number(match[1]),column:Number(match[2])}]:[];
 }).slice(0,3):[];
}
async function nativeFailureDiagnostics(items,states){
 return Promise.all(states.slice(0,3).map(async(state,index)=>{
  const value={index,running:typeof state.status?.running==='boolean'?state.status.running:null,
   mode:['agent','human','private'].includes(state.status?.mode)?state.status.mode:null,
   overflow:state.overflow===true,operations:state.native_operations.map(operation=>({...operation})),
   operations_truncated:state.native_operations_truncated,viewer_state:null};
  for(const key of ['browser_starts','browser_closes','pending_effects','unknown','refused','confirmed'])
   value[key]=Number.isSafeInteger(state[key])&&state[key]>=0?state[key]:null;
  const item=items.find(item=>item.state===state);
  if(item&&!item.page.isClosed()){
   const phase=await viewer(item.page).getAttribute('data-state',{timeout:1000}).catch(()=>null);
   if(['idle','agent','human','private','watching','connecting','recovering','waiting-page','needs-review','switching','stopped','unavailable','disconnected','error'].includes(phase))value.viewer_state=phase;
  }
  return value;
 }));
}
function validate(cfg,{launchers=true}={}){
 assert.equal(cfg.schema,1);
 const native=cfg.native_route||{mode:'public'};assert.ok(['public','local'].includes(native.mode));
 if(native.mode==='local'){assert.deepEqual(Object.keys(native).sort(),['directory','expected_vessel_id','mode']);assert.ok(path.isAbsolute(native.directory)&&native.directory!=='/'&&uuid(native.expected_vessel_id));assert.equal(cfg.access_file,undefined);}
 else {assert.deepEqual(Object.keys(native),['mode']);assert.ok(path.isAbsolute(cfg.access_file));}
 assert.equal(cfg.sessions.length,2);assert.equal(new Set(cfg.sessions.map(s=>s.id)).size,2);
 assert.ok(uuid(cfg.web_connection));https(cfg.console_origin);const socket=new URL(cfg.web_socket);
 assert.equal(socket.protocol,'wss:');assert.equal(socket.pathname,'/v1/vessel/browser-socket');
 assert.ok(!socket.username&&!socket.password&&!socket.search&&!socket.hash&&(!socket.port||socket.port==='443'));
 for(const s of cfg.sessions){assert.ok(uuid(s.id)&&label(s.label)&&s.title.startsWith('qualification-333-'));
  if(launchers)assert.ok(path.isAbsolute(s.native_launcher));}
 assert.equal(new Set(cfg.sessions.map(s=>s.label)).size,2);
 https(cfg.fixture.url);https(cfg.fixture.private.url);
 https(cfg.site_classes.url);
 for(const key of ['ready_selector','asset_selector','shadow_selector','shadow_text','css_color'])assert.ok(typeof cfg.site_classes[key]==='string'&&cfg.site_classes[key].length<=256);
 assert.ok(Number.isInteger(cfg.site_classes.asset_width)&&cfg.site_classes.asset_width>0&&cfg.site_classes.asset_width<=512);
 assert.ok(Array.isArray(cfg.site_classes.frame_texts)&&cfg.site_classes.frame_texts.length===2&&cfg.site_classes.frame_texts.every(v=>typeof v==='string'&&v.length<=128));
 for(const key of ['ready_selector','counter_selector','click_name'])assert.ok(typeof cfg.fixture[key]==='string'&&cfg.fixture[key].length<=256);
 assert.ok(typeof cfg.fixture.private.input_label==='string'&&cfg.fixture.private.input_label.length<=128);
 assert.ok(typeof cfg.fixture.private.ready_selector==='string'&&cfg.fixture.private.ready_selector.length<=256);
 assert.ok(Array.isArray(cfg.sites)&&cfg.sites.length>=3&&cfg.sites.length<=6);
 for(const site of cfg.sites){assert.ok(label(site.label)&&['static','dynamic','frame-media'].includes(site.kind));https(site.url);assert.ok(typeof site.ready_selector==='string'&&site.ready_selector.length<=256);}
 assert.equal(new Set(cfg.sites.map(s=>s.kind)).size,3);
 assert.equal(new Set(cfg.sites.map(s=>s.label)).size,cfg.sites.length);
 assert.ok(path.isAbsolute(cfg.native_reopen_mailbox),'owning native TUI reopen mailbox required');
 assert.ok(path.isAbsolute(cfg.output)&&path.isAbsolute(cfg.chromium)&&path.isAbsolute(cfg.playwright_module));
 assert.ok(cfg.host_observer&&cfg.client_ledger&&cfg.native_wire_evidence,'private measurement dependencies required before effects');
 assert.ok(['playwright','cua'].includes(cfg.web_mode));
 const host=cfg.host_observer;
 if(host.mode==='ssh'){
  assert.ok(/^[A-Za-z0-9._-]{1,253}$/.test(host.hostname)&&/^[A-Za-z0-9_-]{1,48}$/.test(host.user));
  assert.ok(Number.isInteger(host.port)&&host.port>0&&host.port<65536&&path.isAbsolute(host.known_hosts));
 }else if(host.mode==='mailbox'){assert.ok(path.isAbsolute(host.mailbox));}
 else assert.equal(host.mode,'local');
 if(cfg.web_mode==='cua')assert.ok(path.isAbsolute(cfg.cua_mailbox));
 for(const key of ['python','cost_script','probe_script','capacity','ledger_a','ledger_ab'])assert.ok(/^\/[A-Za-z0-9_.\/-]+$/.test(host[key]));
 for(const key of ['cost_sha256','probe_sha256'])assert.ok(/^[a-f0-9]{64}$/.test(host[key]));
 for(const s of cfg.sessions){assert.ok(/^\/[A-Za-z0-9_.\/-]+$/.test(s.host_browser_root));if(native.mode==='local')assert.equal(s.host_browser_root,path.join(native.directory,'sessions',s.id,'journal','host-browser'));}
 if(host.mode==='local'){
  assert.ok(host.programs&&typeof host.programs==='object','qualified local program pins required');
  assert.deepEqual(Object.keys(host.programs).sort(),['guardian','node','python','voyage','worker']);
  for(const program of Object.values(host.programs)){assert.deepEqual(Object.keys(program).sort(),['path','sha256']);assert.ok(/^\/[A-Za-z0-9_.\/-]+$/.test(program.path)&&/^[a-f0-9]{64}$/.test(program.sha256));}
  if(launchers){assert.ok(cfg.native_helm_program);assert.ok(/^\/[A-Za-z0-9_.\/-]+$/.test(cfg.native_helm_program.path)&&/^[a-f0-9]{64}$/.test(cfg.native_helm_program.sha256));}
 }
}
// The authorized host/CUA operator supplies results. A request is issued once,
// paired with its exact private digest and deadline, and never automatically
// resubmitted. This is an orchestration boundary, not an authentication adapter.
async function mailbox(directory,kind,operation,timeout=45000){
 const meta=await fs.lstat(directory);assert.ok(meta.isDirectory()&&meta.uid===process.getuid()&&!(meta.mode&0o077));
 const id=randomUUID(),request={schema:1,id,kind,operation,expires_at_ms:Date.now()+timeout};
 const digest=hash(JSON.stringify(request));await save(path.join(directory,id+'.request.json'),{...request,digest});
 const resultPath=path.join(directory,id+'.response.json');let available=false;
 await until(async()=>{try{await fs.lstat(resultPath);available=true;return true;}catch(error){if(error.code==='ENOENT')return false;throw error;}},timeout);
 assert.ok(available);const reply=await privateJson(resultPath,256*1024);
 assert.equal(reply.schema,1);assert.equal(reply.id,id);assert.equal(reply.digest,digest);assert.equal(reply.status,'observed');
 return reply.result;
}
// This observer retains only selected fixture status/fences, fixed counters and
// command categories. Raw payloads/tokens/URLs/text are never stored or logged.
function nativeReceiptClaim(operation){
 if(!['attach','control','detach'].includes(operation?.action))return null;
 const keys=operation.action==='control'?['action','binding','command_id','mode']:['action','binding','command_id'];
 assert.deepEqual(Object.keys(operation).sort(),keys.sort());assert.ok(uuid(operation.command_id));
 const binding=operation.binding;assert.deepEqual(Object.keys(binding).sort(),['attachment_id','browser_id','capture_epoch','controller_epoch','document_epoch','incarnation','tab_id','viewport_epoch']);
 for(const key of ['attachment_id','browser_id','incarnation','tab_id'])assert.ok(uuid(binding[key]));
 for(const key of ['capture_epoch','controller_epoch','document_epoch','viewport_epoch'])assert.ok(Number.isSafeInteger(binding[key])&&binding[key]>0);
 if(operation.action==='control')assert.ok(['agent','human','private'].includes(operation.mode));
 return {...operation,binding:{...binding}}; // Only typed IDs/epochs/mode, never human input.
}
function observation(page,session,socketUrl,{native=false}={}){
 const state={status:null,snapshot:null,pending:new Map(),effects:new Set(),native_claims:new Map(),browser_starts:0,browser_closes:0,pending_effects:0,confirmed:0,unknown:0,refused:0,
  connections:0,hellos:0,duplicate_effect:false,overflow:false,native_operations:[],native_operations_truncated:false,
  traffic:{sent_bytes:0,received_bytes:0,sent_frames:0,received_frames:0}};
 tracked.set(page,state);
 const accept=(operation,reply)=>{
  if(native&&operation.claim&&reply?.outcome_unknown!==true&&reply?.error==null&&reply?.status?.binding)state.native_claims.set(operation.command_id,operation.claim);
  if(reply?.status==='prepared'&&reply.not_dispatched===true){state.effects.delete(operation.command_id);return;}
  if(reply?.status?.binding)state.status={binding:reply.status.binding,mode:reply.status.mode,
   running:reply.status.running,controller:reply.status.controller,input_sequence:reply.status.input_sequence,
   page_url:reply.status.page?.url||''};
  if(effect(operation.action)){if(reply?.outcome_unknown===true){state.unknown++;}
   else if(reply?.error!=null){state.refused++;}else state.confirmed++;}
 };
 const sent=operation=>{
  if(!operation||typeof operation.action!=='string')return;
  if(effect(operation.action)&&operation.command_id){
   state.pending_effects++;
   if(state.effects.has(operation.command_id))state.duplicate_effect=true;
   state.effects.add(operation.command_id);if(state.effects.size>256)state.overflow=true;
  }
 };
 if(native){
  const operationDiagnostics=new WeakMap();
  page.on('request',request=>{
   const url=new URL(request.url());if(url.pathname!=='/operation'||url.hostname!=='127.0.0.1')return;
   const body=request.postData();if(!body||body.length>256*1024){state.overflow=true;return;}
   try{const op=JSON.parse(body);
    if(state.native_operations.length<16){
     const diagnostic={action:['status','start','attach','control','detach','close','mirror','input','receipt','dialog'].includes(op.action)?op.action:'other',http_status:null};
     state.native_operations.push(diagnostic);operationDiagnostics.set(request,diagnostic);
    }else state.native_operations_truncated=true;
    if(op.action==='start')state.browser_starts++;if(op.action==='close')state.browser_closes++;sent(op);state.pending.set(request,{action:op.action,command_id:op.command_id,claim:nativeReceiptClaim(op)});if(state.pending.size>64)state.overflow=true;}catch{state.overflow=true;}
  });
  page.on('requestfailed',request=>{const op=state.pending.get(request);if(op&&effect(op.action)){state.unknown++;state.pending_effects--;}state.pending.delete(request);});
  page.on('response',async response=>{
   const diagnostic=operationDiagnostics.get(response.request()),status=response.status();
   if(diagnostic&&Number.isInteger(status)&&status>=100&&status<=599)diagnostic.http_status=status;
   const op=state.pending.get(response.request());if(!op)return;
   try{if(!response.ok()){if(effect(op.action))state.unknown++;return;}
    const body=await response.body();if(body.length>4*1024*1024){state.overflow=true;return;}
    accept(op,JSON.parse(body).result);
   }catch{if(effect(op.action))state.unknown++;}
   finally{if(state.pending.delete(response.request())&&effect(op.action))state.pending_effects--;}
  });
 }else page.on('websocket',socket=>{
  if(socket.url()!==socketUrl)return;state.connections++;
  const requests=new Map();
  socket.on('framesent',({payload})=>{
   state.traffic.sent_bytes+=Buffer.byteLength(payload);state.traffic.sent_frames++;
   if(Buffer.byteLength(payload)>4*1024*1024){state.overflow=true;return;}
   const text=payload.toString();if(!text.includes('"command"'))return;
   try{const frame=JSON.parse(text),cmd=frame.request?.command;if(frame.type!=='command'||cmd?.session_id!==session)return;
    if(requests.size>=64){state.overflow=true;return;}
    // No Submit, Goal or ownership-changing command can be part of this pass.
    assert.ok(['host_browser','snapshot','inspect','events','history','decisions','controls','receipt','message_chunk','run_output'].includes(cmd.op));
    const op=cmd.operation;if(op)sent(op);requests.set(frame.request_id,{op:cmd.op,action:op?.action,command_id:op?.command_id});
   }catch{state.overflow=true;}
  });
  socket.on('framereceived',({payload})=>{
   state.traffic.received_bytes+=Buffer.byteLength(payload);state.traffic.received_frames++;
   if(Buffer.byteLength(payload)>4*1024*1024){state.overflow=true;return;}
   const text=payload.toString();if(!text.includes('"reply"')&&!text.includes('"hello"'))return;
   try{const frame=JSON.parse(text);if(frame.type==='hello'){if(frame.protocol===1)state.hellos++;return;}
    const op=requests.get(frame.request_id);if(!op)return;requests.delete(frame.request_id);if(op.action&&effect(op.action))state.pending_effects--;
    const reply=frame.response,env=reply?.result;if(reply?.outcome_unknown!==false||reply?.error!=null){if(op.action&&effect(op.action)){if(reply?.outcome_unknown!==false)state.unknown++;else state.refused++;}return;}
    assert.equal(env?.session_id,session);
    if(op.op==='snapshot'||op.op==='inspect'){const snap=env.result;assert.equal(snap.session_id,session);
     state.snapshot={empty:Array.isArray(snap.messages)&&snap.messages.length===0,no_run:!snap.run,no_pending:!snap.pending_cleanup_run,revision:snap.revision};
    }else if(op.op==='host_browser')accept(op,env.result);
   }catch{state.overflow=true;}
  });
  socket.on('close',()=>{for(const op of requests.values())if(op.action&&effect(op.action)){state.unknown++;state.pending_effects--;}requests.clear();});
 });
 return state;
}
function sshCommand(cfg,args,input=null,timeout=20000){
 const h=cfg.host_observer;
 if(h.mode==='mailbox')return mailbox(h.mailbox,'host_read',{argv:args,stdin:input},Math.max(timeout,45000));
 if(h.mode==='local'){
  assert.ok([h.python,'/usr/bin/sha256sum'].includes(args[0]));
  return new Promise((resolve,reject)=>{
   const child=execFile(args[0],args.slice(1),{timeout,maxBuffer:256*1024},(error,stdout)=>error?reject(Error('fixed local host observation failed')):resolve(stdout));
   child.stdin.end(input===null?'':JSON.stringify(input));
  });
 }
 const argv=['-F','/dev/null','-o','BatchMode=yes','-o','StrictHostKeyChecking=yes','-o','ConnectTimeout=10',
  '-o','UserKnownHostsFile='+h.known_hosts,'-p',String(h.port),'-l',h.user];
 if(h.identity_file)argv.push('-i',h.identity_file,'-o','IdentitiesOnly=yes');
 argv.push('--',h.hostname,args.join(' '));
 return new Promise((resolve,reject)=>{
  const child=execFile('/usr/bin/ssh',argv,{timeout,maxBuffer:256*1024},(error,stdout)=>{
   if(error)reject(Error('fixed read-only host observation failed'));else resolve(stdout);
  });
  child.stdin.end(input===null?'':JSON.stringify(input));
 });
}
async function pinHostHelpers(cfg){
 const h=cfg.host_observer;
 for(const [script,digest] of [[h.cost_script,h.cost_sha256],[h.probe_script,h.probe_sha256]]){
  const reply=await sshCommand(cfg,['/usr/bin/sha256sum','--',script]);assert.equal(reply.slice(0,64),digest);
 }
}
// Same-host automatic ledger derivation happens after actual viewer/bootstrap,
// before window timing. No guessed pre-create PID can qualify the local route.
async function conditionLedger(cfg,condition,nativeItems,providedLedger){
 const h=cfg.host_observer;
 if(h.mode!=='local')return {path:providedLedger,proof:null,scope:'externally prepared host ledger; separate host PID namespace'};
 const selected=condition==='ab_four_viewers'?nativeItems.slice(0,2):nativeItems.slice(0,1);
 assert.equal(selected.length,condition==='ab_four_viewers'?2:1);
 const shape=item=>{
  const binding=item.state.status?.binding;assert.ok(item.state.status?.running&&binding);
  assert.ok(uuid(item.session.id)&&uuid(binding.incarnation)&&uuid(binding.browser_id));
  return {label:item.session.label,session_id:item.session.id,incarnation:binding.incarnation,browser_id:binding.browser_id,root:item.session.host_browser_root};
 };
 const sessions=selected.map(shape);
 await nativeSnapshot(cfg); // Exact live TUI inode/PID/start/transport bootstrap.
 const request={schema:1,capacity:h.capacity,sessions,clients:cfg.client_roots,
  programs:{...h.programs,helm:cfg.native_helm_program}};
 const proof=JSON.parse(await sshCommand(cfg,[h.python,'-I',h.probe_script,'ledger'],request,15000));
 assert.equal(proof.schema,1);assert.equal(proof.scope,'selected current Voyage leaves and browser guardian descendants; excludes unrelated host services');
 assert.ok(Number.isSafeInteger(proof.captured_at_ms)&&Date.now()-proof.captured_at_ms>=0&&Date.now()-proof.captured_at_ms<=1000);
 assert.deepEqual(proof.clients,cfg.client_roots);
 assert.equal(proof.bindings.length,sessions.length);assert.equal(proof.ledger.schema,1);assert.equal(proof.ledger.roots.length,sessions.length*2);
 for(let i=0;i<sessions.length;i++){
  for(const key of ['label','session_id','incarnation','browser_id'])assert.equal(proof.bindings[i][key],sessions[i][key]);
  const [voyage,guardian]=proof.ledger.roots.slice(i*2,i*2+2);
  assert.deepEqual(voyage,{label:'voyage-'+sessions[i].label,pid:proof.bindings[i].voyage.pid,start_ticks:proof.bindings[i].voyage.start_ticks,descendants:false});
  assert.deepEqual(guardian,{label:'browser-'+sessions[i].label,pid:proof.bindings[i].guardian.pid,start_ticks:proof.bindings[i].guardian.start_ticks,descendants:true});
 }
 assert.deepEqual(selected.map(shape),sessions,'native owner/browser changed during ledger preparation');
 const ledgerPath=path.join(cfg.output,'host-ledger-'+condition+'-private.json');
 await save(ledgerPath,proof.ledger);
 await save(path.join(cfg.output,'host-ledger-'+condition+'-proof-private.json'),proof);
 return {path:ledgerPath,proof,scope:proof.scope};
}

const viewer=page=>page.locator('.host-browser-viewer');
const mirror=page=>page.frameLocator('.browser-next-mirror iframe');
async function ready(page){await until(async()=>!tracked.get(page)?.pending_effects&&
 await page.locator('.browser-next-frames').getAttribute('data-control')==='true');}
async function loaded(page,site){await mirror(page).locator(site.ready_selector).waitFor({state:'visible'});}
async function navigate(page,url,marker){await ready(page);const old=tracked.get(page).status.binding.document_epoch;
 await page.getByRole('textbox',{name:'Website address',exact:true}).fill(url);
 await page.getByRole('button',{name:'Go to address',exact:true}).click();
 await until(()=>tracked.get(page).status?.page_url===url&&tracked.get(page).status.binding.document_epoch>old);
 await loaded(page,{ready_selector:marker});await ready(page);}
async function mode(page,name,state){await page.getByRole('button',{name,exact:true}).click();await viewer(page).waitFor({state:'visible'});
 await page.waitForFunction(state=>document.querySelector('.host-browser-viewer')?.dataset.state===state,state);}
async function excluded(page){await page.waitForFunction(()=>document.querySelector('.browser-next-mirror iframe')===null);
 assert.equal(await page.getByRole('textbox',{name:'Website address',exact:true}).inputValue(),'');
 assert.equal(await viewer(page).getAttribute('data-state'),'watching');}
async function count(page,selector,value){await mirror(page).locator(selector).filter({hasText:new RegExp('^'+value+'$')}).waitFor();}
async function fallback(page,selector,minimum=1){
 assert.ok(Number.isInteger(minimum)&&minimum>0&&minimum<=16);
 await until(async()=>{
  const images=viewer(page).locator(selector);if(await images.count()<minimum)return false;
  for(let i=0;i<minimum;i++)if(!await images.nth(i).isVisible()||!await images.nth(i).evaluate(e=>e.tagName==='IMG'&&e.complete&&e.naturalWidth>0))return false;
  return true;
 });
}
// Only the pinned synthetic /media page may disclose these small color reductions.
function ownedMedia(cfg,site){
 const fixture=new URL(cfg.fixture.url),url=new URL(site.url);
 return site.label==='owned-media'&&site.kind==='frame-media'&&url.origin===fixture.origin
  &&url.pathname===fixture.pathname.replace(/\/ui$/,'/media')&&/^\/release-qualification-333\/[A-Za-z0-9_-]{8,48}\/media$/.test(url.pathname);
}
function publicMediaFence(page){
 const state=tracked.get(page);assert.ok(state?.status?.running&&state.status.mode!=='private');
 assert.ok(!state.overflow&&!state.unknown&&!state.refused&&!state.duplicate_effect);
 const b=state.status.binding;assert.ok(b&&uuid(b.incarnation)&&uuid(b.browser_id));
 return JSON.stringify([b.incarnation,b.browser_id,b.tab_id,b.document_epoch,b.capture_epoch,b.controller_epoch]);
}
async function playbackFact(page){
 return mirror(page).locator('#video-state').evaluate(e=>({state:e.dataset.state,frames:Number(e.dataset.frames),time_ms:Number(e.dataset.timeMs),starts:Number(e.dataset.starts)}));
}
async function mediaHandles(page){
 const fence=publicMediaFence(page);await fallback(page,'.browser-next-visual',3);
 const handles={};
 try{for(const [name,selector] of [['canvas','#canvas'],['video','#video'],['unsupported','#unsupported-frame']]){
  const rect=await mirror(page).locator(selector).evaluate(e=>{const r=e.getBoundingClientRect();return {left:Math.max(0,r.left),top:Math.max(0,r.top),width:r.width,height:r.height};});
  const images=viewer(page).locator('.browser-next-visual');
  const indexes=await images.evaluateAll((elements,rect)=>elements.flatMap((e,i)=>Math.abs(parseFloat(e.style.left)-rect.left)<2&&Math.abs(parseFloat(e.style.top)-rect.top)<2&&Math.abs(parseFloat(e.style.width)-rect.width)<2&&Math.abs(parseFloat(e.style.height)-rect.height)<2?[i]:[]),rect);
  assert.equal(indexes.length,1,'exact localized surface unavailable or ambiguous');
  handles[name]=await images.nth(indexes[0]).elementHandle();assert.ok(handles[name]);
 }
 assert.equal(publicMediaFence(page),fence);return {page,fence,handles};
 }catch(error){await Promise.all(Object.values(handles).map(handle=>handle.dispose()));throw error;}
}
async function mediaSample(observed){
 assert.equal(publicMediaFence(observed.page),observed.fence);
 const value={};
 for(const [name,handle] of Object.entries(observed.handles)){
  let sample;
  await until(async()=>{
   assert.equal(publicMediaFence(observed.page),observed.fence);
   sample=await handle.evaluate(e=>{
   if(!e.isConnected||e.tagName!=='IMG')throw Error('localized surface node retired');
   if(!e.complete||e.naturalWidth<8||e.naturalHeight<8)return null;
   if(!e.src.startsWith('data:image/jpeg;base64,')||e.src.length>270000)throw Error('localized image shape unavailable');
   const scratch=e.ownerDocument.createElement('canvas');scratch.width=4;scratch.height=4;
   const context=scratch.getContext('2d',{willReadFrequently:true});if(!context)return null;
   context.drawImage(e,Math.floor(e.naturalWidth/2)-2,Math.floor(e.naturalHeight/2)-2,4,4,0,0,4,4);
   return {version:Number(e.dataset.version),node_id:Number(e.dataset.nodeId),pixels:[...context.getImageData(0,0,4,4).data]};
   });return sample!==null;
  },2000);
  assert.ok(sample,'same decoded localized image node unavailable');
  assert.ok(Number.isSafeInteger(sample.version)&&sample.version>0&&Number.isSafeInteger(sample.node_id)&&sample.node_id>0);
  observed.node_ids??={};observed.node_ids[name]??=sample.node_id;assert.equal(sample.node_id,observed.node_ids[name]);
  value[name]={version:sample.version,color:mediaColor(sample.pixels)};
 }
 value.playback=await playbackFact(observed.page);
 assert.equal(publicMediaFence(observed.page),observed.fence);return value;
}
async function mediaMovement(observed){
 const samples=[],deadline=performance.now()+12000;let facts;
 for(let i=0;i<24&&performance.now()<deadline;i++){
  const sample=await mediaSample(observed);samples.push(sample);
  assert.equal(sample.unsupported.color,'orange','unsupported frame did not decode its synthetic content');
  if(samples.length>=2){try{facts=mediaMovementFacts(samples);}catch{}if(facts)break;}
  await sleep(550);
 }
 assert.ok(facts,'localized canvas/video content or decoded playback did not qualify');
 observed.unsupported=samples.at(-1).unsupported;
 return {...facts,all_three_simultaneously_visible:true,same_dom_image_nodes:true,unsupported_frame_color:'orange',scope:'synthetic localized JPEG samples at worker cadence; not full video cadence or audio'};
}
async function mediaFrameChanged(observed){
 const before=observed.unsupported;assert.ok(before&&before.color==='orange');let latest;
 await until(async()=>{latest=await mediaSample(observed);return latest.unsupported.color==='blue'&&latest.unsupported.version>before.version;},10000);
 assert.equal(await mirror(observed.page).locator('#unsupported-state').getAttribute('data-changes'),'1');
 return {content_changed:true,decoded_colors:['orange','blue'],same_dom_image_node:true,first_version:before.version,last_version:latest.unsupported.version,all_three_simultaneously_visible:true};
}
async function disposeMedia(observed){await Promise.all(Object.values(observed.handles).map(handle=>handle.dispose()));}
async function playOwnedMedia(page){
 await ready(page);await mirror(page).getByRole('button',{name:'Play synthetic video',exact:true}).click();
 await until(async()=>{const value=await playbackFact(page);return value.state==='playing'&&value.starts===1&&value.frames>=3;},10000);
 await ready(page);
}
async function changeOwnedFrame(page){await ready(page);await mirror(page).getByRole('button',{name:'Change unsupported frame',exact:true}).click();await ready(page);}

async function frameContaining(page,text){
 const count=await page.locator('.browser-next-frame iframe').count();assert.ok(count<=8);
 for(let i=0;i<count;i++){const frame=page.frameLocator('.browser-next-frame iframe').nth(i);
  if(await frame.locator('body').evaluate((e,text)=>(e.textContent||'').slice(0,4096).includes(text),text))return frame;
 }
 return null;
}
async function classFacts(page,classes){
 await loaded(page,classes);
 await until(async()=>await mirror(page).locator('body').evaluate(e=>getComputedStyle(e).backgroundColor)===classes.css_color);
 await until(()=>mirror(page).locator(classes.shadow_selector).evaluate((e,text)=>e.shadowRoot?.textContent.includes(text),classes.shadow_text));
 await until(()=>mirror(page).locator(classes.asset_selector).evaluate((e,width)=>e.complete&&e.naturalWidth===width,classes.asset_width));
 for(const text of classes.frame_texts)await until(async()=>!!await frameContaining(page,text));
 return {external_css:true,open_shadow:true,authenticated_image:true,cross_origin_child:true,nested_child:true};
}
async function closeBrowser(page){await ready(page);
 if(await viewer(page).getAttribute('data-state')==='agent'){
  await page.getByLabel('More browser options',{exact:true}).click();await mode(page,'Use browser','human');await ready(page);
 }
 await page.getByLabel('More browser options',{exact:true}).click();
 await page.getByRole('button',{name:'Close browser',exact:true}).click();await page.waitForFunction(()=>document.querySelector('.host-browser-viewer')?.dataset.state==='stopped');}

// Reattach through the initiating Native principal, regardless of Web's grant.
// The old one-use launcher is never reused. This emits one fixed parent request.
async function reclaimNative(context,cfg,binding,openPage=()=>context.newPage()){
 const launcher=await requestNativeReopen(cfg);
 const page=await openPage(),state=observation(page,cfg.sessions[0].id,cfg.web_socket,{native:true});
 const fresh={page,state,native:true,sessionId:cfg.sessions[0].id};
 try{
  await page.goto(pathToFileURL(launcher).href);await viewer(page).waitFor();
  await until(()=>state.status?.running&&state.status.mode==='private');
  await mode(page,'Browse privately','private');await ready(page);
  assert.equal(state.status.binding.browser_id,binding.browser_id);assert.equal(state.status.binding.incarnation,binding.incarnation);
  fresh.private_reclaim=privateReclaimQualified(binding,state.status,state.native_claims);
  return fresh;
 }catch(error){await page.close({runBeforeUnload:false}).catch(()=>{});throw error;}
}

export async function runProductionQualification(context,cfg){
 validate(cfg);context.setDefaultTimeout(30000);
 const report={schema:1,status:'pending',source:'actual production routes; no adapter/mock socket',
  stages:[],matrix:[],native_windows:[],latencies:[],sites:[],cleanup:{},limitations:['Native viewer HTTP request/response payload and native Vessel socket application bytes are separate layers, not total TCP/TLS traffic.',
   'Renderer task duration is not whole-client OS CPU. RSS includes shared pages; PSS apportions them.',
   'Both TUI clients are connected; fixture B browser is opened only for its own viewer condition.']};
 report.transport_context={native:nativeMode(cfg),web:'deployed HTTPS/browser WSS',native_public_tls_requested:nativeMode(cfg)==='public'};
 const pages=[],states=[],closeAttempts=new Set();let resources=null;
 const openOwnedPage=async()=>{assert.ok(pages.length<5,'qualification page bound reached');const page=await context.newPage();pages.push(page);return page;};
 const stage=value=>{report.stage=value;report.stages.push(value);};
 const hostRequest=()=>({schema:1,capacity:cfg.host_observer.capacity,sessions:cfg.sessions.map((s,i)=>({
  label:s.label,session_id:s.id,browser_id:states[i*2].status.binding.browser_id,root:s.host_browser_root}))});
 async function newNative(item){const page=await openOwnedPage();
  const state=observation(page,item.id,cfg.web_socket,{native:true});await page.goto(pathToFileURL(item.native_launcher).href);
  await viewer(page).waitFor();await until(()=>state.status?.running);await ready(page);
  assert.equal(new URL(page.url()).hostname,'127.0.0.1');assert.equal(new URL(page.url()).hash,'');return {page,state};}
 async function newWeb(item){const page=await openOwnedPage();
  const state=observation(page,item.id,cfg.web_socket);await page.goto(new URL(`/voyages/${cfg.web_connection}/${item.id}`,cfg.console_origin).href);
  await until(()=>state.snapshot?.empty&&state.snapshot?.no_run&&state.snapshot?.no_pending);
  assert.ok(await page.locator('.voyage-card[aria-current="true"] .card-title').filter({hasText:item.title}).count()===1);
  await page.getByRole('button',{name:'Browser',exact:true}).first().click();await viewer(page).waitFor();
  await until(()=>state.status?.running);assert.ok(await page.locator('.task-browser-content .host-browser-viewer').count()===1);
  assert.equal(new URL(page.url()).origin,new URL(cfg.console_origin).origin);return {page,state};}
 async function measure(condition,active,ledger){
  stage('cost_'+condition);
  const ledgerEvidence=await conditionLedger(cfg,condition,active.filter(item=>item.native).map(item=>({...item,session:cfg.sessions.find(s=>s.id===item.sessionId)})),ledger);ledger=ledgerEvidence.path;
  await sleep(2000);
  for(let index=0;index<3;index++){
   const started=new Date().toISOString();await save(path.join(cfg.output,`window-${condition}-${index}.json`),{schema:1,condition,index,started});
   const nativeBefore=await nativeSnapshot(cfg);
   const nativeAfter=sleep(10000).then(()=>nativeSnapshot(cfg));
   const costs=await Promise.all(active.map(({page,native})=>browserCost(context,page,{nativeOperationUrls:native?[new URL('/operation',page.url()).href]:[],maxMilliseconds:10000})));
   const trafficBefore=active.map(({state})=>({...state.traffic}));
   const trafficAfter=sleep(10000).then(()=>active.map(({state})=>({...state.traffic})));
   const hostPromise=sshCommand(cfg,[cfg.host_observer.python,'-I',cfg.host_observer.cost_script,'--ledger',ledger,'--seconds','10','--interval','0.25','--output','-'],null,20000);
   const localPromise=new Promise((resolve,reject)=>execFile(cfg.python||'/usr/bin/python3',[
    '-I',path.join(path.dirname(new URL(import.meta.url).pathname),'host_browser_cost.py'),'--ledger',cfg.client_ledger,
    '--seconds','10','--interval','0.25','--output','-'],{timeout:20000,maxBuffer:256*1024},(error,stdout)=>error?reject(Error('owned client measurement failed')):resolve(stdout)));
   const [hostRaw,clientRaw,trafficEnd,nativeEnd]=await Promise.all([hostPromise,localPromise,trafficAfter,nativeAfter]);
   for(let i=0;i<active.length;i++)if(!active[i].native)assert.ok(trafficEnd[i].received_bytes>trafficBefore[i].received_bytes&&active[i].state.connections>0,'actual selected Web payload unavailable');
   const measurements=await Promise.all(costs.map(cost=>cost.stop()));assert.ok(measurements.every(m=>m.status==='observed'));
   report.native_windows.push(nativeWindow(condition,index,nativeBefore,nativeEnd));
   const host=JSON.parse(hostRaw),client=JSON.parse(clientRaw);assert.equal(host.status,'observed');assert.equal(client.status,'observed');
   assert.ok(host.samples.every(s=>s.memory_unavailable===0&&s.zombies===0));assert.ok(client.samples.every(s=>s.memory_unavailable===0&&s.zombies===0));
   const all_window_aligned=measurementWindowAligned(Date.parse(started),[
    {start:host.window_started_at_ms,end:host.window_ended_at_ms},{start:client.window_started_at_ms,end:client.window_ended_at_ms},
    ...report.native_windows.at(-1).clients.map(c=>({start:c.started_at_ms,end:c.ended_at_ms})),
    ...measurements.map(m=>({start:m.started_at_ms,end:m.ended_at_ms}))]);
   report.matrix.push({condition,index,started,host,client,host_ledger_scope:ledgerEvidence.scope,all_window_aligned,host_window_aligned:all_window_aligned,
    viewers:measurements.map((value,i)=>({
    kind:active[i].native?'native':'deployed_react',renderer:value,
    actual_web_application_payload:active[i].native?null:Object.fromEntries(Object.keys(trafficBefore[i]).map(k=>[k,trafficEnd[i][k]-trafficBefore[i][k]]))}))});
  }
 }
 try{
  stage('private_dependencies');await pinHostHelpers(cfg);
  const nativeA=await newNative(cfg.sessions[0]);states.push(nativeA.state);nativeA.native=true;nativeA.sessionId=cfg.sessions[0].id;
  stage('native_a_fixture');await navigate(nativeA.page,cfg.fixture.url,cfg.fixture.ready_selector);await count(nativeA.page,cfg.fixture.counter_selector,0);
  await measure('a_one_viewer',[nativeA],cfg.host_observer.ledger_a);
  const webA=await newWeb(cfg.sessions[0]);states.push(webA.state);await loaded(webA.page,cfg.fixture);
  assert.equal(webA.state.status.binding.browser_id,nativeA.state.status.binding.browser_id);
  await measure('a_two_viewers',[nativeA,webA],cfg.host_observer.ledger_a);
  const nativeB=await newNative(cfg.sessions[1]);states.push(nativeB.state);nativeB.native=true;nativeB.sessionId=cfg.sessions[1].id;
  await navigate(nativeB.page,cfg.fixture.url,cfg.fixture.ready_selector);await count(nativeB.page,cfg.fixture.counter_selector,0);
  const webB=await newWeb(cfg.sessions[1]);states.push(webB.state);await loaded(webB.page,cfg.fixture);
  assert.equal(webB.state.status.binding.browser_id,nativeB.state.status.binding.browser_id);
  assert.notEqual(nativeA.state.status.binding.browser_id,nativeB.state.status.binding.browser_id);
  stage('pin_actual_host_resources');report.native_authority=await localAuthorityProof(cfg,[nativeA,nativeB]);resources=JSON.parse(await sshCommand(cfg,[cfg.host_observer.python,'-I',cfg.host_observer.probe_script,'before'],hostRequest()));
  await save(path.join(cfg.output,'owned-resources-private.json'),resources);
  await measure('ab_four_viewers',[nativeA,webA,nativeB,webB],cfg.host_observer.ledger_ab);
  stage('one_intent_visible_in_both_clients');await ready(nativeA.page);
  report.latencies.push(await inputVisibleLatency(()=>mirror(nativeA.page).getByRole('button',{name:cfg.fixture.click_name,exact:true}).click(),
   ()=>Promise.all([count(nativeA.page,cfg.fixture.counter_selector,1),count(webA.page,cfg.fixture.counter_selector,1)])));
  await count(nativeB.page,cfg.fixture.counter_selector,0);await count(webB.page,cfg.fixture.counter_selector,0);
  stage('owned_site_classes');await navigate(nativeB.page,cfg.site_classes.url,cfg.site_classes.ready_selector);
  report.site_classes={native:await classFacts(nativeB.page,cfg.site_classes),react:await classFacts(webB.page,cfg.site_classes)};
  await ready(nativeB.page);await mirror(nativeB.page).getByRole('button',{name:'Change page',exact:true}).click();
  await until(async()=>await mirror(webB.page).locator('#result').textContent()==='Changed in task browser');await ready(nativeB.page);
  const child=await frameContaining(nativeB.page,cfg.site_classes.frame_texts[0]);
  await child.getByRole('button',{name:'Child action',exact:true}).click();await until(async()=>!!await frameContaining(webB.page,'Child action observed'));
  await mode(nativeB.page,'Continue agent','agent');await ready(webB.page);
  const nested=await frameContaining(webB.page,cfg.site_classes.frame_texts[1]);
  report.latencies.push(await inputVisibleLatency(()=>nested.getByRole('button',{name:'Nested action',exact:true}).click(),
   ()=>until(async()=>!!await frameContaining(nativeB.page,'Nested action observed'))));
  await ready(webB.page);await mode(webB.page,'Continue agent','agent');
  report.site_classes.dynamic_action_both=true;report.site_classes.cross_origin_action_both=true;report.site_classes.nested_action_both=true;
  stage('approved_representative_sites');
  for(const site of cfg.sites){await navigate(nativeB.page,site.url,site.ready_selector);await loaded(webB.page,site);
   let media=null;
   if(ownedMedia(cfg,site)){
    await playOwnedMedia(nativeB.page);const acquired=await Promise.allSettled([mediaHandles(nativeB.page),mediaHandles(webB.page)]);
    if(acquired.some(value=>value.status==='rejected')){
     await Promise.all(acquired.filter(value=>value.status==='fulfilled').map(value=>disposeMedia(value.value)));
     throw acquired.find(value=>value.status==='rejected').reason;
    }
    const views=acquired.map(value=>value.value);
    try{const movement=await Promise.all(views.map(mediaMovement));await changeOwnedFrame(nativeB.page);const unsupported=await Promise.all(views.map(mediaFrameChanged));media={native:{movement:movement[0],unsupported:unsupported[0]},react:{movement:movement[1],unsupported:unsupported[1]}};}
    finally{await Promise.all(views.map(disposeMedia));}
   }else if(site.fallback_selector){await fallback(nativeB.page,site.fallback_selector,site.minimum_fallback_images);await fallback(webB.page,site.fallback_selector,site.minimum_fallback_images);}
   for(const [kind,page] of [['native',nativeB.page],['react',webB.page]])await viewer(page).screenshot({path:path.join(cfg.output,`public-${site.label}-${kind}.png`)});
   report.sites.push({label:site.label,kind:site.kind,url_sha256:hash(site.url),loaded_both:true,localized_fallback:!!site.fallback_selector,media});
  }
  stage('private_handoff');const old={...nativeA.state.status.binding};await mode(nativeA.page,'Browse privately','private');await excluded(webA.page);
  await navigate(nativeA.page,cfg.fixture.private.url,cfg.fixture.private.ready_selector);
  await mirror(nativeA.page).getByLabel(cfg.fixture.private.input_label,{exact:true}).click();await ready(nativeA.page);
  await nativeA.page.getByRole('textbox',{name:'Text for browser',exact:true}).fill('SYNTHETIC_PRODUCTION_PRIVATE_333');
  await nativeA.page.getByRole('button',{name:'Send text to browser',exact:true}).click();
  await until(async()=>await mirror(nativeA.page).getByLabel(cfg.fixture.private.input_label,{exact:true}).inputValue()==='SYNTHETIC_PRODUCTION_PRIVATE_333');
  await excluded(webA.page);const privateBefore=privateBindingSnapshot(nativeA.state.status);
  await nativeA.page.getByRole('button',{name:'Close viewer',exact:true}).click();
  await excluded(webA.page);assert.equal(webA.state.status.mode,'private');
  stage('initiating_native_private_reclaim');
  const freshA=await reclaimNative(context,cfg,privateBefore,openOwnedPage);states.push(freshA.state);
  await excluded(webA.page);
  // Clear under Native private control. Web must remain excluded until return.
  await navigate(freshA.page,cfg.fixture.url,cfg.fixture.ready_selector);await excluded(webA.page);
  await mode(freshA.page,'Continue agent','agent');await ready(freshA.page);
  await mirror(freshA.page).getByRole('button',{name:cfg.fixture.click_name,exact:true}).click();
  await Promise.all([count(freshA.page,cfg.fixture.counter_selector,1),count(webA.page,cfg.fixture.counter_selector,1)]);
  report.native_authority_after_reclaim=await localAuthorityProof(cfg,[freshA,nativeB],report.native_authority);
  report.private={other_client_excluded:true,disconnect_retained_private:true,initiating_native_principal_reclaimed:true,...freshA.private_reclaim,explicit_return:true};
  stage('actual_ticket_renewal');await until(()=>webA.state.hellos>=2&&webB.state.hellos>=2,140000);
  await count(webA.page,cfg.fixture.counter_selector,1);assert.equal(webA.state.status.binding.browser_id,old.browser_id);
  assert.equal(webB.state.status.binding.browser_id,nativeB.state.status.binding.browser_id);
  report.renewal={both_actual_react_connections_renewed:true,no_replayed_intents:states.every(s=>!s.duplicate_effect)};
  stage('explicit_owned_browser_cleanup');closeAttempts.add(0);await closeBrowser(freshA.page);closeAttempts.add(1);await closeBrowser(nativeB.page);
  const request=hostRequest();request.proofs=resources.proofs;
  const cleanup=JSON.parse(await sshCommand(cfg,[cfg.host_observer.python,'-I',cfg.host_observer.probe_script,'after'],request));
  assert.equal(cleanup.sessions.length,2);assert.ok(cleanup.sessions.every(s=>s.complete));report.cleanup.host=cleanup;
  for(const s of states){assert.equal(s.overflow,false);assert.equal(s.unknown,0);assert.equal(s.refused,0);assert.equal(s.duplicate_effect,false);}
  report.native_wire=await saveNativeWire(cfg,report.native_windows);
  report.status=report.matrix.every(w=>w.host_window_aligned)?'passed':'interaction_passed_measurements_incomplete';stage('complete');
 }catch(error){report.status='failed_or_incomplete';report.failure_category=error instanceof assert.AssertionError?'acceptance_not_observed':'bounded_operation_failed';
 }finally{
  report.observation=states.map(s=>({confirmed:s.confirmed,unknown:s.unknown,refused:s.refused,duplicate_effect:s.duplicate_effect,
   scope_overflow:s.overflow,connections:s.connections,hellos:s.hellos}));
  // Unknown close receipts are never replayed. On other failures retain exact
  // owned resource obligations for the operator; closing a page is only detach.
  report.cleanup.close_attempted=[...closeAttempts];report.cleanup.remote_cleanup_unresolved=report.cleanup.host?.sessions?.every(s=>s.complete)!==true;
  let closed=0;for(const page of pages){if(!page.isClosed())await page.close({runBeforeUnload:false}).catch(()=>{});if(page.isClosed())closed++;}
  report.cleanup.owned_pages={created:pages.length,closed,unresolved:pages.length-closed};
  if(closed!==pages.length)report.status='failed_or_incomplete';
  await save(path.join(cfg.output,'production-report.json'),report);
 }
 return report;
}

// Native pages are independently owned fixture Chromium. Web steps execute in
// the already authenticated human browser through CUA, not in these pages. The
// operator must retain the exact CUA and host evidence behind each fixed reply.
export async function runCuaProductionQualification(context,cfg){
 validate(cfg);assert.equal(cfg.web_mode,'cua');context.setDefaultTimeout(30000);
 const report={schema:1,status:'pending',mode:'actual native pages plus authenticated CUA Web tabs',
  matrix:[],native_windows:[],sites:[],latencies:[],cleanup:{},limitations:['CUA Web renderer metrics must be supplied from actual per-tab observation; unavailable metrics do not pass.',
   'Native HTTP bridge payload and native Vessel socket payload are separate layers; local mode is not public WSS/TLS qualification.',
   'Host reports are produced through the separately authorized host conduit; this process performs no service changes.']};
 report.transport_context={native:nativeMode(cfg),web:'deployed HTTPS/browser WSS',native_public_tls_requested:nativeMode(cfg)==='public'};
 const native=[],nativeStates=[],attempted=new Set(),ownedNative=nativePageOwner(context);let resources=null;
 const web=async(operation,fields={})=>mailbox(cfg.cua_mailbox,'cua_web',{operation,...fields});
 const hostRequest=()=>({schema:1,capacity:cfg.host_observer.capacity,sessions:cfg.sessions.map((s,i)=>({
  label:s.label,session_id:s.id,browser_id:native[i].state.status.binding.browser_id,root:s.host_browser_root}))});
 const stage=value=>{report.stage=value;};
 const proof=async(operation,fields,keys)=>{const reply=await web(operation,fields);
  for(const key of keys)assert.equal(reply[key],true);return reply;};
 const localCost=()=>new Promise((resolve,reject)=>execFile(cfg.python||'/usr/bin/python3',[
  '-I',path.join(path.dirname(new URL(import.meta.url).pathname),'host_browser_cost.py'),'--ledger',cfg.client_ledger,
  '--seconds','10','--interval','0.25','--output','-'],{timeout:20000,maxBuffer:256*1024},
  (error,stdout)=>error?reject(Error('owned client measurement failed')):resolve(JSON.parse(stdout))));
 async function measure(condition,pages,webLabels,ledger){
  stage('cost_'+condition);
  const ledgerEvidence=await conditionLedger(cfg,condition,pages.map((item,i)=>({...item,session:cfg.sessions[i]})),ledger);ledger=ledgerEvidence.path;
  for(let index=0;index<3;index++){
   let scheduled=Date.now();
   if(webLabels.length){
    const arm=await web('measure_arm',{condition,index,milliseconds:10000,labels:webLabels});
    assert.ok(Number.isSafeInteger(arm.started_at_ms)&&arm.started_at_ms-Date.now()>=1000&&arm.started_at_ms-Date.now()<=30000);
    scheduled=arm.started_at_ms;
   }
   // Publish the exact requested future window before it starts. Root's CUA
   // collector awaits that time in one bounded call; no background API or
   // retrospectively manufactured metric sample is needed.
   const actualWeb=webLabels.length?web('measure_window',{condition,index,milliseconds:10000,started_at_ms:scheduled,labels:webLabels}):Promise.resolve({viewers:[]});
   await sleep(Math.max(0,scheduled-Date.now()));
   const started=new Date().toISOString();await save(path.join(cfg.output,`window-${condition}-${index}.json`),{schema:1,condition,index,started});
   const nativeBefore=await nativeSnapshot(cfg);
   const nativeAfter=sleep(10000).then(()=>nativeSnapshot(cfg));
   const nativeCosts=await Promise.all(pages.map(item=>browserCost(context,item.page,{nativeOperationUrls:[new URL('/operation',item.page.url()).href],maxMilliseconds:10000})));
   const [hostRaw,client,cua,nativeEnd]=await Promise.all([
    sshCommand(cfg,[cfg.host_observer.python,'-I',cfg.host_observer.cost_script,'--ledger',ledger,'--seconds','10','--interval','0.25','--output','-'],null,20000),
    localCost(),actualWeb,nativeAfter]);
   const host=JSON.parse(hostRaw);assert.equal(host.status,'observed');assert.equal(client.status,'observed');
   report.native_windows.push(nativeWindow(condition,index,nativeBefore,nativeEnd));
   assert.ok(host.samples.every(s=>s.memory_unavailable===0&&s.zombies===0));assert.ok(client.samples.every(s=>s.memory_unavailable===0&&s.zombies===0));
   assert.equal(cua.viewers.length,webLabels.length);
   const summaries=cua.viewers.map((v,i)=>{
    assert.equal(v.label,webLabels[i]);
    if(v.status==='unavailable')return {label:v.label,status:'unavailable',category:'per_tab_measurement_unavailable'};
    assert.equal(v.status,'observed');for(const key of ['duplicate_effects','unknown_effects','refused_effects','pending_effects'])assert.equal(v[key],0);
    assert.equal(v.scope,'actual CUA qualification Web tab renderer and public WSS application payload');
    for(const key of ['task_seconds','heap_used_bytes','sent_bytes','received_bytes'])assert.ok(Number.isFinite(v[key])&&v[key]>=0);
    assert.ok(v.received_bytes>0&&v.selected_connection_observed===true);
    assert.equal(v.truncated,false);
    return {label:v.label,task_seconds:v.task_seconds,heap_used_bytes:v.heap_used_bytes,heap_delta_bytes:v.heap_delta_bytes,
     nodes:v.nodes,node_delta:v.node_delta,metric_scope:v.metric_scope,sent_bytes:v.sent_bytes,received_bytes:v.received_bytes,scope:v.scope,
     window_aligned:Number.isFinite(v.captured_at_ms)&&Math.abs(v.captured_at_ms-Date.parse(started))<1000&&Math.abs(v.elapsed_ms-10000)<1000};
   });
   const nativeRenderer=await Promise.all(nativeCosts.map(c=>c.stop()));assert.ok(nativeRenderer.every(v=>v.status==='observed'));
   const all_window_aligned=measurementWindowAligned(Date.parse(started),[
    {start:host.window_started_at_ms,end:host.window_ended_at_ms},{start:client.window_started_at_ms,end:client.window_ended_at_ms},
    ...report.native_windows.at(-1).clients.map(c=>({start:c.started_at_ms,end:c.ended_at_ms})),
    ...nativeRenderer.map(m=>({start:m.started_at_ms,end:m.ended_at_ms})),
    ...cua.viewers.map(v=>({start:v.captured_at_ms,end:v.captured_at_ms+v.elapsed_ms}))]);
   report.matrix.push({condition,index,started,host,client,host_ledger_scope:ledgerEvidence.scope,all_window_aligned,host_window_aligned:all_window_aligned,
    cua_web:summaries,native:nativeRenderer});
  }
 }
 try{
  stage('pin_read_only_host_helpers');await pinHostHelpers(cfg);
  // Both TUI clients opened their exact fixture browser. Load only A's one-use
  // launcher first; B is excluded from the first two host-ledger conditions.
  stage('native_a_page_create');
  const first=await ownedNative.open(),firstState=observation(first,cfg.sessions[0].id,cfg.web_socket,{native:true});native.push({page:first,state:firstState});nativeStates.push(firstState);
  stage('native_a_launcher_load');await first.goto(pathToFileURL(cfg.sessions[0].native_launcher).href);
  stage('native_a_viewer_visible');await viewer(first).waitFor();
  stage('native_a_running');await until(()=>firstState.status?.running);
  stage('native_a_control_ready');await ready(first);
  stage('native_a_fixture_navigate');await navigate(native[0].page,cfg.fixture.url,cfg.fixture.ready_selector);
  stage('native_a_counter_ready');await count(native[0].page,cfg.fixture.counter_selector,0);
  await measure('a_one_viewer',[native[0]],[],cfg.host_observer.ledger_a);
  stage('cua_web_a_admission');await proof('open_fixture',{label:cfg.sessions[0].label,
   url:new URL(`/voyages/${cfg.web_connection}/${cfg.sessions[0].id}`,cfg.console_origin).href,
   title:cfg.sessions[0].title,fixture_selector:cfg.fixture.ready_selector,counter_selector:cfg.fixture.counter_selector,counter:0},
   ['authenticated_existing_session','exact_selected_voyage','browser_dock_open','fixture_visible','counter_matches']);
  await measure('a_two_viewers',[native[0]],[cfg.sessions[0].label],cfg.host_observer.ledger_a);
  stage('native_b_page_create');
  const page=await ownedNative.open(),state=observation(page,cfg.sessions[1].id,cfg.web_socket,{native:true});native.push({page,state});nativeStates.push(state);
  stage('native_b_launcher_load');await page.goto(pathToFileURL(cfg.sessions[1].native_launcher).href);
  stage('native_b_viewer_visible');await viewer(page).waitFor();
  stage('native_b_running');await until(()=>state.status?.running);
  stage('native_b_control_ready');await ready(page);
  stage('native_b_fixture_navigate');await navigate(page,cfg.fixture.url,cfg.fixture.ready_selector);
  stage('native_b_counter_ready');await count(page,cfg.fixture.counter_selector,0);
  stage('native_b_distinct_browser');
  assert.notEqual(state.status.binding.browser_id,native[0].state.status.binding.browser_id);
  stage('cua_web_b_admission');await proof('open_fixture',{label:cfg.sessions[1].label,
   url:new URL(`/voyages/${cfg.web_connection}/${cfg.sessions[1].id}`,cfg.console_origin).href,
   title:cfg.sessions[1].title,fixture_selector:cfg.fixture.ready_selector,counter_selector:cfg.fixture.counter_selector,counter:0},
   ['authenticated_existing_session','exact_selected_voyage','browser_dock_open','fixture_visible','counter_matches','other_web_tab_retained']);
  report.native_authority=await localAuthorityProof(cfg,native);
  resources=JSON.parse(await sshCommand(cfg,[cfg.host_observer.python,'-I',cfg.host_observer.probe_script,'before'],hostRequest()));
  await save(path.join(cfg.output,'owned-resources-private.json'),resources);
  await measure('ab_four_viewers',native,cfg.sessions.map(s=>s.label),cfg.host_observer.ledger_ab);
  stage('native_intent_seen_by_cua_web');await ready(native[0].page);
  report.latencies.push(await inputVisibleLatency(()=>mirror(native[0].page).getByRole('button',{name:cfg.fixture.click_name,exact:true}).click(),
   ()=>count(native[0].page,cfg.fixture.counter_selector,1)));
  await proof('observe_counter',{label:cfg.sessions[0].label,selector:cfg.fixture.counter_selector,value:1},['counter_matches','no_input_sent']);
  await count(native[1].page,cfg.fixture.counter_selector,0);
  await proof('observe_counter',{label:cfg.sessions[1].label,selector:cfg.fixture.counter_selector,value:0},['counter_matches','no_input_sent']);
  stage('owned_site_classes');await navigate(native[1].page,cfg.site_classes.url,cfg.site_classes.ready_selector);
  report.site_classes={native:await classFacts(native[1].page,cfg.site_classes)};
  await proof('observe_site_classes',{label:cfg.sessions[1].label,classes:cfg.site_classes},
   ['external_css','open_shadow','authenticated_image','cross_origin_child','nested_child','no_input_sent']);
  await ready(native[1].page);await mirror(native[1].page).getByRole('button',{name:'Change page',exact:true}).click();
  await until(async()=>await mirror(native[1].page).locator('#result').textContent()==='Changed in task browser');await ready(native[1].page);
  const child=await frameContaining(native[1].page,cfg.site_classes.frame_texts[0]);await child.getByRole('button',{name:'Child action',exact:true}).click();
  await until(async()=>!!await frameContaining(native[1].page,'Child action observed'));
  await mode(native[1].page,'Continue agent','agent');
  await proof('nested_child_once_return',{label:cfg.sessions[1].label},
   ['dynamic_action_observed','cross_origin_action_observed','nested_action_dispatched_once','nested_action_observed','continue_agent_explicit','no_unknown_effect_retried']);
  await until(async()=>!!await frameContaining(native[1].page,'Nested action observed'));
  report.site_classes.dynamic_action_both=true;report.site_classes.cross_origin_action_both=true;report.site_classes.nested_action_both=true;
  stage('approved_public_sites');
  for(const site of cfg.sites){await navigate(native[1].page,site.url,site.ready_selector);
   let media=null;
   if(ownedMedia(cfg,site)){
    await playOwnedMedia(native[1].page);const observed=await mediaHandles(native[1].page);
    try{const movement=await mediaMovement(observed);
     await proof('observe_media_surfaces',{label:cfg.sessions[1].label},['all_three_visible','canvas_decoded_content_changes','video_decoded_content_changes','video_playback_frames_advance','same_surface_versions_advance','unsupported_frame_orange','no_input_sent']);
     await changeOwnedFrame(native[1].page);const unsupported=await mediaFrameChanged(observed);
     await proof('observe_media_frame_change',{label:cfg.sessions[1].label},['all_three_visible','unsupported_frame_blue','same_unsupported_surface_version_advance','no_input_sent']);
     media={native:{movement,unsupported},react:{decoded_visual_proofs_observed:true,no_input_sent:true}};
    }finally{await disposeMedia(observed);}
   }else{
    await proof('observe_public_site',{label:cfg.sessions[1].label,site_label:site.label,selector:site.ready_selector,
     fallback_selector:site.fallback_selector||null},['fixture_visible','no_input_sent']);
    if(site.fallback_selector)await fallback(native[1].page,site.fallback_selector,site.minimum_fallback_images);
   }
   await viewer(native[1].page).screenshot({path:path.join(cfg.output,`public-${site.label}-native.png`)});
   report.sites.push({label:site.label,kind:site.kind,url_sha256:hash(site.url),visible_both:true,localized_fallback:!!site.fallback_selector,media});
  }
  stage('private_cua_exclusion');await mode(native[0].page,'Browse privately','private');
  await proof('observe_private_exclusion',{label:cfg.sessions[0].label},['no_replay_iframe','address_blank','watching_private','no_input_sent']);
  await navigate(native[0].page,cfg.fixture.private.url,cfg.fixture.private.ready_selector);
  await mirror(native[0].page).getByLabel(cfg.fixture.private.input_label,{exact:true}).click();await ready(native[0].page);
  await native[0].page.getByRole('textbox',{name:'Text for browser',exact:true}).fill('SYNTHETIC_PRODUCTION_PRIVATE_333');
  await native[0].page.getByRole('button',{name:'Send text to browser',exact:true}).click();
  await until(async()=>await mirror(native[0].page).getByLabel(cfg.fixture.private.input_label,{exact:true}).inputValue()==='SYNTHETIC_PRODUCTION_PRIVATE_333');
  await proof('observe_private_exclusion',{label:cfg.sessions[0].label},['no_replay_iframe','address_blank','watching_private','no_input_sent']);
  const privateBefore=privateBindingSnapshot(native[0].state.status);
  await native[0].page.getByRole('button',{name:'Close viewer',exact:true}).click();
  await proof('observe_private_exclusion',{label:cfg.sessions[0].label},['no_replay_iframe','address_blank','watching_private','no_input_sent']);
  stage('initiating_native_private_reclaim');
  const freshA=await reclaimNative(context,cfg,privateBefore,ownedNative.open);nativeStates.push(freshA.state);native[0]=freshA;
  await proof('observe_private_exclusion',{label:cfg.sessions[0].label},['no_replay_iframe','address_blank','watching_private','no_input_sent']);
  await navigate(freshA.page,cfg.fixture.url,cfg.fixture.ready_selector);
  await proof('observe_private_exclusion',{label:cfg.sessions[0].label},['no_replay_iframe','address_blank','watching_private','no_input_sent']);
  await mode(freshA.page,'Continue agent','agent');await ready(freshA.page);
  await mirror(freshA.page).getByRole('button',{name:cfg.fixture.click_name,exact:true}).click();await count(freshA.page,cfg.fixture.counter_selector,1);
  await proof('observe_counter',{label:cfg.sessions[0].label,selector:cfg.fixture.counter_selector,value:1},['counter_matches','no_input_sent']);
  report.native_authority_after_reclaim=await localAuthorityProof(cfg,native,report.native_authority);
  report.private={other_client_excluded:true,disconnect_retained_private:true,initiating_native_principal_reclaimed:true,...freshA.private_reclaim,explicit_return:true};
  stage('actual_cua_renewal');await proof('observe_real_renewal',{labels:cfg.sessions.map(s=>s.label)},
   ['both_actual_connections_renewed','browser_identity_retained','no_effect_replay','no_transport_rewrite']);
stage('zero_viewer_capture_pause');
const idleItem=native[0],idlePrior={...idleItem.state.status.binding};
const idleLedger=await conditionLedger(cfg,'a_idle',[{...idleItem,session:cfg.sessions[0]}],cfg.host_observer.ledger_a);
const idleClaims=new Set(idleItem.state.native_claims.keys());
const idleEffects={browser_starts:idleItem.state.browser_starts,browser_closes:idleItem.state.browser_closes};
await proof('close_dock_for_idle',{label:cfg.sessions[0].label},
 ['dock_closed','qualification_tab_retained','other_browser_unchanged','no_browser_start_or_close']);
await idleItem.page.getByLabel('More browser options',{exact:true}).click();
await idleItem.page.getByRole('button',{name:'Disconnect viewer',exact:true}).click();
await until(()=>[...idleItem.state.native_claims.entries()].some(([id,claim])=>!idleClaims.has(id)&&claim.action==='detach'));
const idleRequest=hostRequest();idleRequest.proofs=resources.proofs;
idleRequest.label=cfg.sessions[0].label;idleRequest.worker_program=cfg.host_observer.programs.worker;
const idleStarted=Date.now();
const [idleProof,idleCost]=await Promise.all([
 sshCommand(cfg,[cfg.host_observer.python,'-I',cfg.host_observer.probe_script,'idle'],idleRequest,20000).then(JSON.parse),
 sshCommand(cfg,[cfg.host_observer.python,'-I',cfg.host_observer.cost_script,'--ledger',idleLedger.path,'--seconds','10','--interval','0.25','--output','-'],null,20000).then(JSON.parse),
]);
const idleEvidence=idleWindowQualified(idleProof,{label:cfg.sessions[0].label});
assert.equal(idleCost.status,'observed');
assert.ok(measurementWindowAligned(idleStarted,[{start:idleCost.window_started_at_ms,end:idleCost.window_ended_at_ms},{start:idleProof.started_at_ms,end:idleProof.ended_at_ms}]));
assert.ok(idleCost.samples.every(s=>s.memory_unavailable===0&&s.zombies===0));
await idleItem.page.getByRole('button',{name:'Check browser status',exact:true}).click();await ready(idleItem.page);
const idleNewClaims=[...idleItem.state.native_claims.entries()].filter(([id])=>!idleClaims.has(id)).map(([,claim])=>claim);
const idleReconnected=idleReconnectQualified(idlePrior,idleItem.state.status,idleNewClaims,idleEffects,idleItem.state);
await proof('reopen_dock_after_idle',{label:cfg.sessions[0].label},
 ['dock_open','same_running_browser','fresh_attach','other_browser_unchanged','no_browser_start_or_close']);
report.zero_viewer_idle={...idleEvidence,...idleReconnected,host:idleCost,host_ledger_scope:idleLedger.scope,active_windows_unchanged:report.matrix.length===9};

  stage('explicit_owned_browser_close');attempted.add(0);await closeBrowser(native[0].page);
  attempted.add(1);await closeBrowser(native[1].page);
  const request=hostRequest();request.proofs=resources.proofs;
  const cleanup=JSON.parse(await sshCommand(cfg,[cfg.host_observer.python,'-I',cfg.host_observer.probe_script,'after'],request));
  assert.equal(cleanup.sessions.length,2);assert.ok(cleanup.sessions.every(s=>s.complete));report.cleanup.host=cleanup;
  await proof('close_fixture_panels',{labels:cfg.sessions.map(s=>s.label)},['only_owned_fixture_tabs_closed','unrelated_tabs_retained']);
  report.native_wire=await saveNativeWire(cfg,report.native_windows);
  for(const state of nativeStates){assert.equal(state.unknown,0);assert.equal(state.refused,0);assert.equal(state.duplicate_effect,false);assert.equal(state.overflow,false);}
  const measurementsComplete=report.matrix.every(w=>w.host_window_aligned&&w.cua_web.every(v=>v.status!=='unavailable'&&v.window_aligned));
  report.status=measurementsComplete&&report.zero_viewer_idle?.observed===true?'passed':'interaction_passed_measurements_incomplete';stage('complete');
 }catch(error){report.status='failed_or_incomplete';report.failure_category=error instanceof assert.AssertionError?'acceptance_not_observed':'bounded_operation_failed';
  report.failure_locations=failureLocations(error);
  report.native_failure_diagnostics=await nativeFailureDiagnostics(native,nativeStates);
 }finally{
  report.cleanup.close_attempted=[...attempted];report.cleanup.remote_cleanup_unresolved=report.cleanup.host?.sessions?.every(s=>s.complete)!==true;
  report.cleanup.native_pages=await ownedNative.close();
  if(report.cleanup.native_pages.unresolved_pages!==0)report.status='failed_or_incomplete';
  await save(path.join(cfg.output,'production-report.json'),report);
 }
 return report;
}

if(process.argv[1]&&pathToFileURL(path.resolve(process.argv[1])).href===import.meta.url){
 const preflight=process.argv[2]==='--validate',cfg=await privateJson(process.argv[preflight?3:2]);validate(cfg,{launchers:!preflight});
 if(preflight){process.stdout.write('private configuration schema accepted; no runtime effects\n');}
 else {
 const {chromium}=await import(pathToFileURL(cfg.playwright_module).href);
 const server=await chromium.launchServer({executablePath:cfg.chromium,headless:true,chromiumSandbox:true});
 const browser=await chromium.connect(server.wsEndpoint());
 try{
  if(cfg.client_ledger==='automatic'){
   const pid=server.process().pid,raw=await fs.readFile(`/proc/${pid}/stat`,'utf8'),fields=raw.slice(raw.lastIndexOf(')')+1).trim().split(/\s+/);
   const ledger={schema:1,roots:[...(cfg.client_roots||[]),{label:'qualification-chromium',pid,start_ticks:Number(fields[19]),descendants:true}]};
   cfg.client_ledger=path.join(cfg.output,'client-ledger-private.json');await save(cfg.client_ledger,ledger);
  }
  const storageState=cfg.web_mode==='playwright'?await privateJson(cfg.storage_state):undefined;
  const context=await browser.newContext({storageState,viewport:{width:1440,height:960},acceptDownloads:false});
  const report=cfg.web_mode==='cua'?await runCuaProductionQualification(context,cfg):await runProductionQualification(context,cfg);
  process.exitCode=report.status==='passed'?0:1;
 }finally{await browser.close();await server.close();}
 }
}
