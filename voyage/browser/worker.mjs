import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID, createHash } from 'node:crypto';
import { gzipSync, gunzipSync } from 'node:zlib';
import {createCssCodec} from './css-transport.mjs';
import { chromium } from 'playwright-core';
import { Journal, UUID, privateDir } from './journal.mjs';
import { Refusal, refuse, digest, origin, networkProxy } from './security.mjs';

// Host-validated literals only. Page-side parameter destructuring/iterators can
// observe argument-array values, so the private key never travels in that form.
const cssCodec=createCssCodec();
export function decodeCssMirror(value,maxBytes=cssCodec.limit){
  if(value?.format!=='css_chunks_v1'||value.encoding!=='gzip-chunks'||!Array.isArray(value.chunks)||!value.chunks.length||value.chunks.length>cssCodec.maxChunks||!Number.isSafeInteger(value.total_bytes)||value.total_bytes<=0||value.total_bytes>Math.min(cssCodec.limit,maxBytes))refuse('mirror_limit');
  const buffer=Buffer.alloc(value.total_bytes);let offset=0;
  for(const text of value.chunks){
    if(typeof text!=='string'||text.length>2200000||!/^([A-Za-z0-9+/]{4})*([A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(text))refuse('mirror_limit');
    const chunk=gunzipSync(Buffer.from(text,'base64'),{maxOutputLength:cssCodec.chunkBytes});
    if(!chunk.length||offset+chunk.length>buffer.length)refuse('mirror_limit');chunk.copy(buffer,offset);offset+=chunk.length;
  }
  if(offset!==buffer.length)refuse('mirror_limit');
  return cssCodec.unpack(JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(buffer)),maxBytes);
}
export function encodeCssMirror(events){
  const packed=cssCodec.pack(events),buffer=Buffer.from(JSON.stringify(packed.payload));
  if(buffer.length>cssCodec.limit)refuse('mirror_limit');
  const chunks=[];for(let offset=0;offset<buffer.length;offset+=cssCodec.chunkBytes)chunks.push(gzipSync(buffer.subarray(offset,offset+cssCodec.chunkBytes),{level:3}).toString('base64'));
  return {value:{encoding:'gzip-chunks',format:'css_chunks_v1',chunks,total_bytes:buffer.length},expanded_bytes:packed.expanded_bytes};
}

export function recorderExpression(operation,key,generation,cursor=0,budget=2200000,format=null){
  if(!UUID.test(key)||!Number.isSafeInteger(generation)||generation<0||!Number.isSafeInteger(cursor)||cursor<0||!Number.isSafeInteger(budget)||budget<0||budget>2200000)refuse('invalid_capture_permit');
  const target='this.__voyageMirror',literal=JSON.stringify(key);
  if(operation==='stop')return `${target}?${target}.stop(${literal},${generation}):({recorder_absent:true})`;
  if(operation!=='drain')refuse('invalid_capture_operation');
  if(format!==null&&format!=='css_chunks_v1')refuse('unsupported_format');
  const method=format==='css_chunks_v1'?'drainCss':'drain';
  return `${target}&&${target}.enable(${literal},${generation})?${target}.${method}(${cursor},${budget},${literal},${generation}):({error:'recorder_disabled'})`;
}

class BeforeEffect extends Refusal {}
const beforeEffect = code => { throw new BeforeEffect(code); };
const exposed = (e,point=false) => {
  // Inline controls can have several line fragments. Their aggregate rectangle
  // may have its center in whitespace or another element. Inspect at most 32
  // visible fragments; never treat an overlapping element as the target.
  const bounds=e.getBoundingClientRect(),rects=e.getClientRects();
  if(!bounds.width||!bounds.height)return false;
  for(let i=0;i<Math.min(32,rects.length);i++){
    const r=rects[i],left=Math.max(0,r.left),top=Math.max(0,r.top);
    const right=Math.min(innerWidth,r.right),bottom=Math.min(innerHeight,r.bottom);
    if(right<=left||bottom<=top)continue;
    const x=(left+right)/2,y=(top+bottom)/2;
    let hit=document.elementFromPoint(x,y);
    for(let depth=0;depth<32&&hit?.shadowRoot;depth++){
      const next=hit.shadowRoot.elementFromPoint(x,y);if(!next||next===hit)break;hit=next;
    }
    for(let node=hit;node;node=node.parentElement||node.getRootNode()?.host){
      if(node===e)return point?{x:(x-bounds.left)/bounds.width,y:(y-bounds.top)/bounds.height}:true;
    }
  }
  return false;
};

// Runs only through the private task browser pipe. Collection and handle lookup
// share this exact bounded list; Playwright's shadow-piercing selector order must
// never be mixed with a light-DOM query's indices.
const agentObservation = ({offset,limit,textOffset}) => {
  const selector='a,button,input,textarea,select,[role="button"],[role="checkbox"],[role="radio"],[role="combobox"],[contenteditable="true"],[draggable="true"]';
  let state=globalThis.__voyageAgentObservation;
  if(!state?.refresh){
    state={version:0,controls:[],roots:[]};
    state.observer=new MutationObserver(()=>state.version++);
    state.refresh=()=>{
      if(state.observer.takeRecords().length)state.version++;
      const controls=[],roots=[document],stack=document.documentElement?[{node:document.documentElement,depth:0}]:[];
      let visited=0,shadowTruncated=false;
      while(stack.length&&visited<100000){
        const {node,depth}=stack.pop();visited++;
        if(node.matches(selector))controls.push(node);
        // Queue one sibling/child at a time so a wide document cannot allocate
        // an unbounded pending list before the traversal budget takes effect.
        if(node.nextElementSibling)stack.push({node:node.nextElementSibling,depth});
        if(node.firstElementChild)stack.push({node:node.firstElementChild,depth});
        if(node.shadowRoot){
          if(depth>=32){shadowTruncated=true;continue;}
          roots.push(node.shadowRoot);
          if(node.shadowRoot.firstElementChild)stack.push({node:node.shadowRoot.firstElementChild,depth:depth+1});
        }
      }
      const rootsChanged=roots.length!==state.roots.length||roots.some((root,i)=>root!==state.roots[i]);
      if(rootsChanged||controls.length!==state.controls.length||controls.some((control,i)=>control!==state.controls[i]))state.version++;
      if(rootsChanged){
        state.observer.disconnect();
        for(const root of roots)state.observer.observe(root,{subtree:true,childList:true,attributes:true,characterData:true});
      }
      state.roots=roots;state.controls=controls;state.nodesTruncated=stack.length>0;state.shadowTruncated=shadowTruncated;
    };
    globalThis.__voyageAgentObservation=state;
  }
  state.refresh();
  if(!document.body)throw new Error('body unavailable');
  // Reading innerText or control geometry forces rendered layout. Do not cross
  // the node budget by laying out an explicitly truncated document afterwards.
  // Such an observation has partial metadata but no grounded effect references.
  const body=state.nodesTruncated?'':document.body.innerText;
  return {version:state.version,text:body.slice(textOffset,textOffset+16384),text_total:body.length,text_truncated:state.nodesTruncated||textOffset+16384<body.length,
    control_total:state.controls.length,controls_truncated:state.nodesTruncated||state.shadowTruncated,
    nodes_truncated:state.nodesTruncated,
    indices:state.nodesTruncated?[]:state.controls.slice(offset,offset+limit).map((_,i)=>offset+i)};
};

const controlSignature=e=>JSON.stringify({connected:e.isConnected,tag:e.tagName,attributes:[...e.attributes].slice(0,128).map(a=>[a.name,a.value.slice(0,16384)]),text:(e.innerText||'').slice(0,1024),disabled:e.disabled,readonly:e.readOnly,checked:e.checked,value:typeof e.value==='string'?e.value.slice(0,16384):null,optionCount:e.options?.length,options:e.tagName==='SELECT'?[...e.options].slice(0,64).map(o=>[o.value,o.selected,o.disabled]):null});
const safeLocation=url=>{try{const u=new URL(url);u.username='';u.password='';return u.href.slice(0,8192);}catch{return '';}};
const MAX_TEXT=16384, MAX_BYTES=2*1024*1024;
const MAX_MAIN_VISUALS=3, MAIN_VISUAL_DEADLINE_MS=4500;
const text=(v,max=MAX_TEXT)=>{if(typeof v!=='string'||Buffer.byteLength(v)>max)refuse('invalid_text');return v;};
const beforeText=(v,max=MAX_TEXT)=>{try{return text(v,max);}catch(e){if(e instanceof Refusal)beforeEffect(e.code);throw e;}};
const number=(v,min,max)=>{if(!Number.isInteger(v)||v<min||v>max)refuse('invalid_number');return v;};
const identifier=v=>{if(typeof v!=='string'||!UUID.test(v))refuse('invalid_id');return v;};
const bounded=async(p,ms=12000)=>{let timer;try{return await Promise.race([p,new Promise((_,r)=>{timer=setTimeout(()=>r(new Refusal('operation_timeout')),ms);})]);}finally{clearTimeout(timer);}};

export class Worker {
  constructor(){
    this.captureObservationSequence=0;this.captureActivityGeneration=0;this.recorderStopObserved=false;this.capturePublication=Promise.resolve();this.captureTasks=new Set();this.cssReads=new Map();this.captureTaskUncertain=false;this.recorderCapability=randomUUID();this.recorderSessionCleanups=new Set();
    this.browser=randomUUID();this.epochs={tab:1,document:1,viewport:1,control:1,capture:1};
    this.mode='agent';this.controller=null;this.viewers=new Map();this.tabs=new Map();this.refs=new Map();this.refBindings=new WeakMap();this.agentFrames=new Map();this.downloads=new Map();this.assets=new Map();this.assetBytes=0;this.assetEffects=new Set();this.assetEpoch=0;this.assetRevision=0;this.cssBatchCache=new Map();this.cssCacheBytes=0;
    this.ordinary=Promise.resolve();this.urgent=Promise.resolve();this.admission=Promise.resolve();this.ephemeral=new Map();this.pending=0;this.fenceWaiters=new Set();this.closing=false;this.disconnected=false;this.effects=new Set();this.metadata=new Map();this.agentActive=0;this.agentAction=null;this.agentCursor=null;this.visuals=[];this.visualAt=0;this.frameVisuals=new Map();this.frameIds=new WeakMap();this.pageErrors=new WeakMap();
  }
  status(){return {mirror_formats:['css_chunks_v1'],viewport:{width:this.config?.width,height:this.config?.height},agent_action:this.mode==='agent'?this.agentAction:null,agent_cursor:this.mode==='agent'?this.agentCursor:null,agent_active:this.agentActive>0,page:this.metadata.get(this.active)||null,tab_details:[...this.tabs.keys()].map(id=>({id,...(this.metadata.get(id)||{})})),dialog:this.dialog?{type:this.dialog.type(),message:this.dialog.message().slice(0,1024)}:null,downloads:[...this.downloads].filter(([,d])=>d.owner===this.controller).map(([id,d])=>({id,name:d.name})),browser:this.browser,epochs:{...this.epochs},mode:this.mode,controller:this.controller,tabs:[...this.tabs.keys()],tab:this.active||null,viewers:[...this.viewers.keys()],open:!!this.task};}
  exact(req){const epochs={...req.epochs};if(['join','mirror','disconnect'].includes(req.op)){epochs.document=this.epochs.document;epochs.viewport=this.epochs.viewport;}if(req.browser!==this.browser||digest(epochs)!==digest(this.epochs))refuse('stale_binding');}
  invalidate(){for(const h of this.refs.values())void h.dispose().catch(()=>{});this.refs.clear();this.agentFrames.clear();}
  advance(...keys){this.clearCssBatches();for(const k of keys)this.epochs[k]++;if(keys.some(k=>['tab','document','viewport','control','capture'].includes(k))){this.visuals=[];this.visualAt=0;this.frameVisuals.clear();}this.invalidate();}
  checkAgent(){if(this.mode!=='agent')refuse('agent_fenced');}
  guard(stamp){if(stamp!==this.epochs.control)refuse('control_fenced');}
  async request(req){
    let id=req?.id;
    try{
      if(this.disconnected)refuse('parent_disconnected');
      if(!req||typeof req!=='object'||Array.isArray(req))refuse('invalid_request');identifier(id);text(req.op,32);
      if(this.pending>=64)refuse('queue_full');this.pending++;
      try{
        let scheduled;
        const admit=this.admission.then(async()=>{
          if(this.disconnected)refuse('parent_disconnected');
          if(req.op==='init'){
            if(this.journal){const old=this.journal.previous(req);if(old){scheduled=Promise.resolve({receipt:old,content_withheld:true});return;}refuse('already_initialized');}
            await this.init(req.config);const r=await this.journal.begin(req);await this.journal.finish(r,'completed');scheduled=Promise.resolve({status:this.status()});return;
          }
          if(!this.journal)refuse('not_initialized');
          const ephemeral=['input','mirror','status','receipt'].includes(req.op);
          const old=ephemeral?this.ephemeral.get(req.id):this.journal.previous(req);
          if(old){if(old.digest!==digest(req))refuse('id_conflict');scheduled=Promise.resolve({receipt:old,content_withheld:true});return;}
          const r=ephemeral?{id:req.id,digest:digest(req),state:'dispatched',ephemeral:true}:await this.journal.begin(req);
          if(ephemeral){this.ephemeral.set(req.id,r);if(this.ephemeral.size>2048)this.ephemeral.delete(this.ephemeral.keys().next().value);}
          const urgent=['control','disconnect','close','shutdown','policy','status'].includes(req.op)||(req.op==='input'&&(req.claim===true||req.action?.kind==='dialog'||(req.action?.kind==='history'&&req.action.direction==='stop')));
          const execute=()=>this.execute(req,r);
          if(req.op==='mirror')scheduled=execute();
          else {const lane=urgent?'urgent':'ordinary';scheduled=this[lane].then(execute);this[lane]=scheduled.catch(()=>{});}
        });
        this.admission=admit.catch(()=>{});await admit;
        return {id,ok:true,result:await scheduled};
      }finally{this.pending--;}
    }catch(e){return {id:typeof id==='string'?id:null,ok:false,error:{code:e instanceof Refusal?e.code:'worker_error',state:e.receiptState||'unknown'}};}
  }
  async init(config){
    if(!config||!path.isAbsolute(config.root||'')||!path.isAbsolute(config.executable||''))refuse('invalid_config');
    this.config={...config,width:number(config.width??1280,320,3840),height:number(config.height??720,240,2160)};
    this.setPolicy(config);
    this.captureProgramSha256=createHash('sha256').update(await fs.readFile(fileURLToPath(import.meta.url))).digest('hex');
    await privateDir(config.root);
    const lockPath=path.join(config.root,'worker.lock');
    try{this.lock=await fs.open(lockPath,'wx',0o600);}catch{refuse('state_locked');}
    this.lockPath=lockPath;await this.lock.writeFile(JSON.stringify({pid:process.pid,browser:this.browser}));await this.lock.sync();
    try{this.journal=new Journal(path.join(config.root,'receipts'));await this.journal.init();}catch(e){this.journal=null;await this.releaseLock();throw e;}
  }
  setPolicy(config){
    if(typeof config.public_web!=='boolean'||!Array.isArray(config.origins)||config.origins.length>128)refuse('invalid_policy');
    const allowed=new Map();for(const g of config.origins){if(!g||origin(g.origin)!==g.origin||typeof g.private_network!=='boolean')refuse('invalid_policy');allowed.set(g.origin,Object.freeze({...g}));}
    this.allowed=allowed;this.publicWeb=config.public_web;
  }
  async execute(req,r){
    let dispatched=false;
    try{
      if(!['status','receipt','open','shutdown'].includes(req.op))this.exact(req);
      if(this.disconnected)refuse('parent_disconnected');
      if(this.closing&&!['status','receipt','shutdown'].includes(req.op))refuse('closing');
      if(req.op==='agent')this.checkAgent();
      dispatched=true;
      const value=await this.dispatch(req);
      if(req.op!=='mirror')await this.refreshMetadata();await this.finishReceipt(r,'completed');return {status:this.status(),value};
    }catch(e){
      const code=e instanceof Refusal?e.code:'outcome_unknown';
      const state=!dispatched||e instanceof BeforeEffect?'refused':'unknown';
      await this.finishReceipt(r,state,code);const error=new Refusal(code);error.receiptState=state;throw error;
    }
  }
  async refreshMetadata(){
    if(!this.context||this.dialog)return;
    const stamp=this.epochs.document,control=this.epochs.control;
    for(const [id,page]of this.tabs){
      let url='';try{const u=new URL(page.url());if(['http:','https:'].includes(u.protocol)){u.username='';u.password='';url=u.href.slice(0,8192);}else if(u.href==='about:blank')url='about:blank';}catch{}
      const meta={url,title:this.metadata.get(id)?.title||'New tab',loading:this.metadata.get(id)?.loading||false,can_go_back:false,can_go_forward:false};
      // CDP metadata remains responsive even while a website modal blocks input.
      let cdp;try{cdp=await bounded(this.context.newCDPSession(page),1000);const h=await bounded(cdp.send('Page.getNavigationHistory'),1000);meta.can_go_back=h.currentIndex>0;meta.can_go_forward=h.currentIndex<h.entries.length-1;meta.title=(h.entries[h.currentIndex]?.title||'New tab').slice(0,256);}catch{}finally{if(cdp)await bounded(cdp.detach(),500).catch(()=>{});}
      if(stamp!==this.epochs.document||control!==this.epochs.control)return;
      meta.loading=this.metadata.get(id)?.loading||false;this.metadata.set(id,meta);
    }
  }
  async finishReceipt(r,state,code){if(r.ephemeral){r.state=state;if(code)r.code=code;}else await this.journal.finish(r,state,code);}
  async dispatch(req){
    switch(req.op){
      case 'status':await this.refreshMetadata();return this.status();
      case 'receipt':identifier(req.request_id);return this.ephemeral.get(req.request_id)||this.journal.records.get(req.request_id)||null;
      case 'open':return this.open();
      case 'close':return this.close();
      case 'shutdown':await this.close();await this.releaseLock();return {shutdown:true};
      case 'policy':this.setPolicy(req);await this.fence();this.proxy?.update(this.allowed,this.publicWeb);return null;
      case 'join':identifier(req.viewer);if(this.viewers.has(req.viewer))refuse('viewer_exists');if(this.viewers.size>=4)refuse('viewer_limit');if(this.mode==='private'&&this.controller!==req.viewer)refuse('private');if(!await this.publishCaptureObservation({joining:true}))beforeEffect('observation_unavailable');this.viewers.set(req.viewer,{seq:0,mirrorCursor:0,frameCursors:new Map()});if(this.viewers.size===1)this.captureActivityGeneration++;this.recorderStopObserved=false;return null;
      case 'disconnect':{
        identifier(req.viewer);this.viewers.delete(req.viewer);
        if(this.controller===req.viewer){if(this.mode!=='private'){this.mode='agent';this.controller=null;}await this.fence();}
        if(!this.viewers.size)await this.stopMirrors();else await this.publishCaptureObservation();return null;
      }
      case 'control':{
        this.viewer(req.viewer);if(!['agent','human','private'].includes(req.mode))refuse('invalid_mode');
        if(this.controller&&this.controller!==req.viewer)refuse('controller_busy');
        if(req.mode==='agent'&&this.controller!==req.viewer)refuse('not_controller');
        this.mode=req.mode;this.controller=req.mode==='agent'?null:req.viewer;
        if(req.mode==='private')for(const id of this.viewers.keys())if(id!==req.viewer)this.viewers.delete(id);
        await this.fence([...this.viewers.keys()]);
        return null;
      }
      case 'mirror':{const task=this.mirror(req);this.captureTasks.add(task);try{return await task;}catch(error){if(error.code==='operation_timeout')this.captureTaskUncertain=true;throw error;}finally{this.captureTasks.delete(task);}}
      case 'input':{
        if(req.claim===true)return this.claimInput(req);
        const effect=this.input(req);this.effects.add(effect);try{return await effect;}finally{this.effects.delete(effect);}
      }
      case 'agent':this.agentActive++;this.agentAction=['inspect','read','diagnostics','navigate','history','click','double_click','fill','key','select','check','drag','scroll','tabs','screenshot','upload','download'].includes(req.action?.kind)?req.action.kind:null;try{return await this.agent(req.action);}finally{this.agentActive--;this.agentAction=null;}
      default:refuse('unknown_operation');
    }
  }
  viewer(id){identifier(id);if(!this.viewers.has(id))refuse('viewer_missing');if(this.mode==='private'&&this.controller!==id)refuse('private');return this.viewers.get(id);}
  requireOpen(){if(!this.task||!this.page||this.page.isClosed())refuse('browser_closed');}
  async open(){
    if(this.task)return null;
    this.proxy=await networkProxy(this.allowed,this.publicWeb);
    const base={executablePath:this.config.executable,headless:true,chromiumSandbox:true,timeout:15000,args:['--disable-background-networking','--disable-component-update','--disable-sync','--disable-quic','--force-webrtc-ip-handling-policy=disable_non_proxied_udp','--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1']};
    try{
      this.task=await chromium.launch({...base,proxy:{server:this.proxy.server,username:this.proxy.username,password:this.proxy.password,bypass:'<-loopback>'}});
      if(this.disconnected)refuse('parent_disconnected');
      this.task.on('disconnected',()=>{this.recorderStopObserved=false;void this.publishCaptureObservation();});
      this.context=await this.task.newContext({viewport:{width:this.config.width,height:this.config.height},acceptDownloads:true,serviceWorkers:'block'});
      this.context.setDefaultTimeout(5000);this.context.setDefaultNavigationTimeout(10000);
      await this.context.route('**/*',async route=>{
        try{const url=route.request().url();if(url==='about:blank')return route.continue();const o=origin(url);if(!this.publicWeb&&!this.allowed.has(o))return route.abort();return route.continue();}catch{return route.abort().catch(()=>{});}
      });
      // WebSockets use Chromium's authenticated proxy, including DNS-pinned
      // CONNECT checks; never forward via a Node-side WebSocket client.
      await this.context.addInitScript(()=>{Object.defineProperty(globalThis,'RTCPeerConnection',{value:undefined,configurable:false});Object.defineProperty(globalThis,'webkitRTCPeerConnection',{value:undefined,configurable:false});});
      this.context.on('page',p=>this.registerPage(p));
      const directory=path.dirname(fileURLToPath(import.meta.url));
      const vendor=await fs.readFile(path.join(directory,'rrweb-vendor.mjs'),'utf8');
      const recorder=(await fs.readFile(path.join(directory,'mirror-source.mjs'),'utf8')).replace('__VOYAGE_CAPTURE_KEY__',this.recorderCapability).replaceAll('__VOYAGE_CSS_CODEC__',`(${createCssCodec.toString()})`);
      await this.context.addInitScript({content:`${vendor}\n;${recorder}`});
      const page=await this.context.newPage();await this.select(this.idFor(page));
      return null;
    }catch(e){await this.close();throw e;}
  }
  idFor(page){for(const [id,p]of this.tabs)if(p===page)return id;}
  registerPage(page){
    if(this.tabs.size>=16){void page.close();return;}
    const id=randomUUID();this.tabs.set(id,page);this.pageErrors.set(page,{console:0,page:0});
    page.on('console',m=>{if(this.mode==='agent'&&m.type()==='error')this.pageErrors.get(page).console=Math.min(100000,this.pageErrors.get(page).console+1);});
    page.on('pageerror',()=>{if(this.mode==='agent')this.pageErrors.get(page).page=Math.min(100000,this.pageErrors.get(page).page+1);});
    page.on('request',request=>{if(request.isNavigationRequest()&&request.frame()===page.mainFrame())this.metadata.set(id,{...this.metadata.get(id),loading:true});});
    page.on('load',()=>{this.metadata.set(id,{...this.metadata.get(id),loading:false});});
    page.on('dialog',dialog=>{if(page===this.page)this.dialog=dialog;else void dialog.dismiss().catch(()=>{});});
    page.on('response',response=>{const effect=this.cacheAsset(response);this.assetEffects.add(effect);void effect.finally(()=>this.assetEffects.delete(effect));});
    page.on('framenavigated',frame=>{this.frameIds.set(frame,randomUUID());if(page===this.page&&frame===page.mainFrame()){this.advance('document');this.dialog=null;}else if(page===this.page)this.invalidate();});
    page.on('close',()=>{this.metadata.delete(id);this.tabs.delete(id);if(this.page===page){this.page=null;this.active=null;this.advance('tab','document');this.recorderStopObserved=false;void this.publishCaptureObservation();}});
    page.on('download',download=>{void this.recordDownload(download);});
  }
  async recordDownload(download){
    const stamp=this.epochs.control;
    const owner=this.mode==='agent'?'agent':this.controller;
    try{const file=await download.path();if(!file)return;const s=await fs.stat(file);if(s.size<=MAX_BYTES&&stamp===this.epochs.control&&this.downloads.size<8){const data=await fs.readFile(file);if(stamp===this.epochs.control&&owner===(this.mode==='agent'?'agent':this.controller))this.downloads.set(randomUUID(),{owner,name:download.suggestedFilename(),data_base64:data.toString('base64')});}}catch{}finally{await download.delete().catch(()=>{});}
  }
  async cacheAsset(response){
    const epoch=this.assetEpoch;
    const type=response.request().resourceType();
    if(!['image','font','stylesheet'].includes(type))return;
    const headers=response.headers(),length=Number(headers['content-length']);
    if(!Number.isSafeInteger(length)||length<0||length>700000)return;
    const mime=(headers['content-type']||'').split(';')[0].toLowerCase();
    if(!/^(?:image\/(?:png|jpeg|webp|gif|svg\+xml|avif)|font\/(?:woff|woff2|ttf|otf)|text\/css|application\/(?:font-woff|font-woff2|vnd\.ms-fontobject))$/.test(mime))return;
    try{
      const body=await response.body();if(body.length>700000||epoch!==this.assetEpoch)return;
      const url=response.url();if(this.assets.has(url))return;
      const data=`data:${mime};base64,${body.toString('base64')}`;
      while(this.assetBytes+data.length>12000000&&this.assets.size){const first=this.assets.keys().next().value;this.assetBytes-=this.assets.get(first).length;this.assets.delete(first);this.assetRevision++;this.clearCssBatches();}
      if(data.length<=12000000){this.assets.set(url,data);this.assetBytes+=data.length;this.assetRevision++;this.clearCssBatches();}
    }catch{}
  }
  inlineAssets(events,base){
    const resource=value=>{
      if(typeof value!=='string'||value.startsWith('data:')||value.startsWith('#'))return value;
      try{return this.assets.get(new URL(value,base).href)||'';}catch{return '';}
    };
    const css=value=>typeof value==='string'?value.replace(/url\(\s*(['"]?)([^)'"\s]+)\1\s*\)/gi,(_,quote,url)=>`url("${resource(url)}")`):value;
    const visit=node=>{
      if(!node||typeof node!=='object')return;
      if(node.attributes&&typeof node.attributes==='object'){
        const a=node.attributes;
        for(const name of ['src','poster'])if(typeof a[name]==='string')a[name]=resource(a[name]);
        if(typeof a.srcset==='string')delete a.srcset;
        if(typeof a.style==='string')a.style=css(a.style);
        if(typeof a._cssText==='string'){
          let stylesheet=base;
          if(node.tagName==='link'&&typeof a.href==='string')try{stylesheet=new URL(a.href,base).href;}catch{}
          a._cssText=a._cssText.replace(/url\(\s*(['"]?)([^)'"\s]+)\1\s*\)/gi,(_,quote,url)=>{
            try{return `url("${this.assets.get(new URL(url,stylesheet).href)||''}")`;}catch{return 'url("")';}
          });
        }
        // A replayed stylesheet must never load from the Helm machine. rrweb's
        // captured CSS text is authoritative; unknown resources stay blank.
        if(node.tagName==='link')delete a.href;
        if(node.tagName==='base')delete a.href;
        if(node.tagName==='iframe')delete a.srcdoc;
        if(node.tagName==='meta'&&String(a['http-equiv']||'').toLowerCase()==='refresh')delete a.content;
      }
      for(const value of Object.values(node))if(value&&typeof value==='object'){
        if(Array.isArray(value))for(const item of value)visit(item);else visit(value);
      }
    };
    for(const event of events)visit(event);
  }
  async captureVisuals(page,mirrored=new Set()){
    if(Date.now()-this.visualAt<1000)return this.visuals.filter(item=>!mirrored.has(item.id));
    this.visualAt=Date.now();
    const deadline=this.visualAt+MAIN_VISUAL_DEADLINE_MS;
    const targets=await page.evaluate(({hidden,limit})=>[...document.querySelectorAll('canvas,video,iframe')].slice(0,12).map(element=>{
      const rect=element.getBoundingClientRect(),style=getComputedStyle(element);
      const left=Math.max(0,rect.left),top=Math.max(0,rect.top);
      return {id:globalThis.__voyageMirror?.id(element),left,top,x:left,y:top,
        width:Math.max(0,Math.min(innerWidth,rect.right)-left),height:Math.max(0,Math.min(innerHeight,rect.bottom)-top),
        visible:style.visibility!=='hidden'&&style.display!=='none'&&rect.width>8&&rect.height>8&&rect.right>0&&rect.bottom>0&&rect.left<innerWidth&&rect.top<innerHeight};
    }).filter(item=>item.visible&&item.id>0&&!hidden.includes(item.id)).sort((a,b)=>b.width*b.height-a.width*a.height).slice(0,limit),{hidden:[...mirrored],limit:MAX_MAIN_VISUALS});
    const visuals=[];
    for(const target of targets){
      if(Date.now()>=deadline)break;
      if(mirrored.has(target.id))continue;
      try{
        const clip={x:Math.max(0,target.x),y:Math.max(0,target.y),
          width:Math.min(target.width,this.config.width),height:Math.min(target.height,this.config.height)};
        if(clip.width<8||clip.height<8)continue;
        const timeout=()=>Math.max(1,Math.min(1500,deadline-Date.now()));
        let content=await page.screenshot({type:'jpeg',quality:55,clip,timeout:timeout()});
        if(content.length>200000&&Date.now()<deadline)content=await page.screenshot({type:'jpeg',quality:30,clip,timeout:timeout()});
        if(content.length<=200000)visuals.push({id:target.id,left:target.left,top:target.top,width:clip.width,height:clip.height,version:this.visualAt,data_base64:content.toString('base64')});
      }catch{}
    }
    this.visuals=visuals;return visuals;
  }
  async captureFrameVisuals(frame,frameId){
    const cached=this.frameVisuals.get(frameId);
    if(cached&&Date.now()-cached.at<1000)return cached.items;
    const targets=await frame.evaluate(()=>[...document.querySelectorAll('canvas,video')].slice(0,8).map(element=>{
      const r=element.getBoundingClientRect(),style=getComputedStyle(element);
      return {id:globalThis.__voyageMirror?.id(element),visible:r.width>8&&r.height>8&&r.right>0&&r.bottom>0&&r.left<innerWidth&&r.top<innerHeight&&style.display!=='none'&&style.visibility!=='hidden'};
    }).filter(item=>item.visible&&item.id>0).slice(0,1));
    const items=[];
    for(const target of targets){
      let handle;
      try{
        handle=await frame.evaluateHandle(id=>globalThis.__voyageMirror?.node(id)??null,target.id);
        const element=handle.asElement();if(!element)continue;
        const box=await element.boundingBox();
        // An element screenshot can scroll the task page as a side effect.
        // A mirror read must capture only the already visible viewport.
        if(!box||box.x<0||box.y<0||box.x+box.width>this.config.width||box.y+box.height>this.config.height)continue;
        const clip={x:box.x,y:box.y,width:box.width,height:box.height};
        let content=await this.page.screenshot({type:'jpeg',quality:55,clip,timeout:1500});
        if(content.length>200000)content=await this.page.screenshot({type:'jpeg',quality:30,clip,timeout:1500});
        if(content.length<=200000)items.push({id:target.id,version:Date.now(),data_base64:content.toString('base64')});
      }catch{}finally{await handle?.dispose().catch(()=>{});}
    }
    this.frameVisuals.set(frameId,{at:Date.now(),items});
    return items;
  }
  async select(id,stamp=this.epochs.control){this.guard(stamp);const page=this.tabs.get(id);if(!page)refuse('tab_missing');await page.setViewportSize({width:this.config.width,height:this.config.height});this.guard(stamp);await page.bringToFront();this.guard(stamp);await this.stopMirrors();this.guard(stamp);this.page=page;this.active=id;this.dialog=null;this.advance('tab','document','capture');}
  frameId(frame){let id=this.frameIds.get(frame);if(!id){id=randomUUID();this.frameIds.set(frame,id);}return id;}
  // Pinned Playwright 1.63 local adapter supplies browser-native frame identity.
  // No identity is obtained from website JS, URLs or sibling ordering. The only
  // CDP expression is our recorder operation, in that exact default context.
  async openRecorderSession(target){
    const opening=this.context.newCDPSession(target);
    try{return await bounded(opening,1000);}
    catch(error){
      if(error.code==='operation_timeout'){
        this.captureTaskUncertain=true;
        // The original creation can finish after the observation deadline.
        // Retain that ownership obligation and detach its exact late result.
        const cleanup=opening.then(session=>bounded(session.detach(),1000));
        this.recorderSessionCleanups.add(cleanup);
        void cleanup.catch(()=>{this.captureTaskUncertain=true;}).finally(()=>this.recorderSessionCleanups.delete(cleanup));
      }
      throw error;
    }
  }
  async recorderCall(frame,operation,cursor=0,budget=2200000,generation=this.captureActivityGeneration,format=null){
    let session;const contexts=new Map();let stale=false,selected=null;
    try{
      const connection=frame?._connection;
      if(typeof connection?.toImpl!=='function')refuse('observation_unavailable');
      const implementation=connection.toImpl(frame),page=frame.page();
      if(!implementation||implementation._page!==connection.toImpl(page)||implementation._page.browserContext!==connection.toImpl(this.context)||typeof implementation._id!=='string'||frame.isDetached())refuse('observation_unavailable');
      const frameId=implementation._id;
      for(let target=frame;target;target=target.parentFrame()){
        try{session=await this.openRecorderSession(target===page.mainFrame()?page:target);break;}
        catch(error){if(!String(error.message).includes('does not have a separate CDP session'))throw error;}
      }
      if(!session)refuse('observation_unavailable');
      const contains=(tree,id)=>tree.frame?.id===id||(tree.childFrames||[]).some(child=>contains(child,id));
      const created=event=>{const context=event.context;if(context.auxData?.isDefault&&context.auxData.frameId===frameId)contexts.set(context.id,context);};
      const destroyed=event=>{if(selected?.id===event.executionContextId)stale=true;contexts.delete(event.executionContextId);};
      session.on('Runtime.executionContextCreated',created);session.on('Runtime.executionContextDestroyed',destroyed);session.on('Runtime.executionContextsCleared',()=>{stale=true;contexts.clear();});
      const before=await bounded(session.send('Page.getFrameTree'),1000);if(!contains(before.frameTree,frameId))refuse('observation_unavailable');
      await bounded(session.send('Runtime.enable'),1000);
      if(contexts.size!==1)refuse('observation_unavailable');selected=[...contexts.values()][0];
      if(typeof selected.uniqueId!=='string'||!selected.uniqueId)refuse('observation_unavailable');
      const expression=recorderExpression(operation,this.recorderCapability,generation,cursor,budget,format);
      const result=await bounded(session.send('Runtime.evaluate',{expression,uniqueContextId:selected.uniqueId,returnByValue:true,awaitPromise:true,timeout:750}),1000);
      const after=await bounded(session.send('Page.getFrameTree'),1000);
      if(stale||frame.isDetached()||implementation._id!==frameId||contexts.get(selected.id)?.uniqueId!==selected.uniqueId||!contains(after.frameTree,frameId)||result.exceptionDetails)refuse('observation_unavailable');
      return result.result?.value;
    }finally{if(session)await bounded(session.detach(),1000).catch(()=>{this.captureTaskUncertain=true;});}
  }
  async stopMirrors(){
    this.clearCssBatches();
    this.recorderStopObserved=false;
    const generation=++this.captureActivityGeneration;
    let retired=true;try{await bounded(Promise.allSettled([...this.captureTasks]),5000);}catch{retired=false;}
    const results=await Promise.allSettled([...this.tabs.values()].filter(page=>!page.isClosed()&&typeof page.frames==='function').flatMap(page=>page.frames().map(frame=>bounded(this.recorderCall(frame,'stop',0,2200000,generation),1000))));
    this.recorderStopObserved=retired&&!this.captureTaskUncertain&&this.cssCacheBytes===0&&this.cssBatchCache.size===0&&this.cssReads.size===0&&generation===this.captureActivityGeneration&&results.length>0&&results.every(result=>result.status==='fulfilled'&&(result.value?.recorder_absent===true||(result.value?.recording===false&&result.value?.pending_events===0&&result.value?.pending_bytes===0)));
    await this.publishCaptureObservation();
    this.frameVisuals.clear();
    for(const viewer of this.viewers.values()){viewer.mirrorCursor=0;viewer.frameCursors.clear();}
  }
  // Private metadata only; never a receipt/admission or public topology field.
  // Failure leaves proof unavailable and cannot turn a control effect into replay.
  async publishCaptureObservation({joining=false}={}){
    if(!this.lock||!this.config?.root)return true;
    const root=this.config.root,destination=path.join(root,'capture-observation.json');
    const temporary=destination+'.'+randomUUID();
    const publish=this.capturePublication.then(async()=>{
    try{
      await privateDir(root);
      const existing=await fs.lstat(destination).catch(error=>{if(error.code!=='ENOENT')throw error;return null;});
      if(existing&&(!existing.isFile()||existing.isSymbolicLink()||existing.nlink!==1||(existing.mode&0o077)||existing.uid!==process.getuid()))throw Error('private observation unavailable');
      const value={schema:1,pid:process.pid,browser:this.browser,source_sha256:this.captureProgramSha256,sequence:++this.captureObservationSequence,
        observed_at_ms:Date.now(),browser_running:!!this.task&&typeof this.task.isConnected==='function'&&this.task.isConnected()&&!!this.page&&!this.page.isClosed(),zero_viewers:!joining&&this.viewers.size===0,
        recorder_stop_observed:!joining&&this.viewers.size===0&&this.recorderStopObserved};
      const fd=await fs.open(temporary,'wx',0o600);
      try{await fd.writeFile(JSON.stringify(value));await fd.sync();}finally{await fd.close();}
      await fs.rename(temporary,destination);return true;
    }catch{this.recorderStopObserved=false;await fs.unlink(temporary).catch(()=>{});return false;}
    });
    this.capturePublication=publish.catch(()=>{});return await publish;
  }
  clearCssBatches(){this.cssBatchCache.clear();this.cssCacheBytes=0;}
  prepareCssBatch(value,frame,url,generation,stamp,maxBytes){
    const key=digest([this.browser,this.active,this.frameId(frame),generation,stamp,this.epochs.control,this.mode,this.assetEpoch,this.assetRevision,url,
      value.encoding,value.format,value.cursor,value.reset,value.latest,value.total_bytes,value.chunks]);
    const cached=this.cssBatchCache.get(key);
    if(cached){if(cached.expanded_bytes>maxBytes||cached.value.total_bytes>maxBytes)refuse('mirror_limit');return cached;}
    let prepared;
    try{
      const decoded=decodeCssMirror(value,maxBytes);this.inlineAssets(decoded.events,url);prepared=encodeCssMirror(decoded.events);
      if(prepared.expanded_bytes>maxBytes||prepared.value.total_bytes>maxBytes)refuse('mirror_limit');
    }catch{refuse('mirror_limit');}
    Object.freeze(prepared.value.chunks);Object.freeze(prepared.value);Object.freeze(prepared);
    const size=Buffer.byteLength(JSON.stringify(prepared.value));
    // One worker-wide bounded cache; entries retain encoded bytes only.
    if(size<=2800000){
      while(this.cssCacheBytes+size>2800000&&this.cssBatchCache.size){const first=this.cssBatchCache.keys().next().value;this.cssCacheBytes-=this.cssBatchCache.get(first).cache_bytes;this.cssBatchCache.delete(first);}
      const entry=Object.freeze({...prepared,cache_bytes:size});this.cssBatchCache.set(key,entry);this.cssCacheBytes+=size;return entry;
    }
    return prepared;
  }
  async recorderRead(frame,since,budget,generation,format,deadline,guard,reads){
    for(;;){
      guard();const remaining=deadline-Date.now();if(remaining<=0)refuse('observation_unavailable');
      const call=Promise.resolve().then(()=>this.recorderCall(frame,'drain',since,budget,generation,format));
      reads?.add(call);this.captureTasks.add(call);
      void call.finally(()=>this.captureTasks.delete(call)).catch(()=>{});
      try{
        const value=await bounded(call,remaining);
        guard();if(value?.error!=='observation_unavailable')return value;
      }catch(error){if(error.code==='operation_timeout'){this.captureTaskUncertain=true;throw error;}guard();if(error.code!=='observation_unavailable')throw error;}
      const wait=Math.min(20,deadline-Date.now());if(wait<=0)refuse('observation_unavailable');
      await new Promise(resolve=>setTimeout(resolve,wait));guard();
    }
  }
  async mirrorRead(frame,since,budget,generation,format,limit,guard,producerGuard=guard){
    const deadline=Date.now()+limit;guard();if(Date.now()>=deadline)refuse('observation_unavailable');
    const captured={context:this.context,page:this.page,document:this.epochs.document,capture:this.epochs.capture,control:this.epochs.control};
    const sharedGuard=()=>{producerGuard();if(captured.context!==this.context||captured.page!==this.page||captured.document!==this.epochs.document||captured.capture!==this.epochs.capture||captured.control!==this.epochs.control||generation!==this.captureActivityGeneration||frame.isDetached?.())refuse('capture_fenced');};
    if(format!=='css_chunks_v1')return this.recorderRead(frame,since,budget,generation,format,deadline,guard);
    const identity=value=>value&&typeof value==='object'?this.frameId(value):null;
    const key=JSON.stringify([identity(this.context),identity(this.page),identity(frame),this.browser,this.active,
      this.epochs.document,this.epochs.capture,this.epochs.control,generation,since,budget,format]);
    let entry=this.cssReads.get(key);
    if(!entry){
      if(this.cssReads.size>=4)refuse('capture_busy');
      entry={reads:new Set(),promise:null};this.cssReads.set(key,entry);
      const owned=entry;
      entry.promise=Promise.resolve().then(async()=>{
        try{
          const value=await this.recorderRead(frame,since,budget,generation,format,deadline,sharedGuard,owned.reads);
          // CSS drain contains encoded strings/scalars only. Legacy event arrays
          // remain per-reader because asset rewriting mutates those arrays.
          if(!value||typeof value!=='object'||Array.isArray(value))refuse('mirror_limit');
          for(const [field,item]of Object.entries(value)){
            if(field==='chunks'){if(!Array.isArray(item)||item.length>16||item.some(chunk=>typeof chunk!=='string'))refuse('mirror_limit');}
            else if(item!==null&&!['string','number','boolean'].includes(typeof item))refuse('mirror_limit');
          }
          const immutable={...value};if(Array.isArray(value.chunks))immutable.chunks=Object.freeze([...value.chunks]);
          return Object.freeze(immutable);
        }finally{
          // A caller's deadline never abandons a late CDP operation. Keep this
          // producer owned until the original call and detach actually retire.
          await Promise.allSettled([...owned.reads]);
          if(this.cssReads.get(key)===owned)this.cssReads.delete(key);
        }
      });
      this.captureTasks.add(entry.promise);
      void entry.promise.finally(()=>this.captureTasks.delete(entry.promise)).catch(()=>{});
    }
    const remaining=deadline-Date.now();if(remaining<=0)refuse('observation_unavailable');
    try{const value=await bounded(entry.promise,remaining);guard();return value;}
    catch(error){if(error.code==='operation_timeout'){this.captureTaskUncertain=true;throw error;}guard();throw error;}
  }
  async mirror(req){
    const viewer=this.viewer(req.viewer);this.requireOpen();
    this.recorderStopObserved=false;
    const generation=this.captureActivityGeneration;
    const captureGuard=()=>{if(generation!==this.captureActivityGeneration||!this.viewers.size)refuse('capture_fenced');this.viewer(req.viewer);};
    const format=req.format??null;if(format!==null&&format!=='css_chunks_v1')refuse('unsupported_format');
    const since=number(req.since,0,Number.MAX_SAFE_INTEGER),page=this.page,stamp=this.epochs.capture;
    // Chromium pauses page evaluation while a JavaScript dialog is open.
    // Keep the read channel responsive so the human can dismiss that dialog.
    if(this.dialog){viewer.mirrorCursor=since;viewer.frameCursors.clear();return {encoding:'gzip',data_base64:gzipSync(Buffer.from('[]')).toString('base64'),cursor:since,reset:false,latest:since,visuals:[],frames:[]};}
    if(!since&&this.assetEffects.size)await bounded(Promise.allSettled([...this.assetEffects]),1000).catch(()=>{});
    captureGuard();
    const producerGuard=()=>{if(generation!==this.captureActivityGeneration||!this.viewers.size||stamp!==this.epochs.capture||page!==this.page)refuse('capture_fenced');};
    const guarded=()=>{captureGuard();producerGuard();};
    const value=await this.mirrorRead(page.mainFrame(),since,2200000,generation,format,5000,guarded,producerGuard);
    if(stamp!==this.epochs.capture||page!==this.page)refuse('capture_fenced');
    this.viewer(req.viewer);
    if(value?.error)refuse(value.error);
    let logical=0,compactTotal=0,mainEncoded=null;
    if(format==='css_chunks_v1'){const prepared=this.prepareCssBatch(value,page.mainFrame(),page.url(),generation,stamp,cssCodec.limit);logical=prepared.expanded_bytes;mainEncoded=prepared.value;compactTotal=mainEncoded.total_bytes;}
    else this.inlineAssets(value.events,page.url());
    const frameCursors=!value.reset&&since===viewer.mirrorCursor?new Map(viewer.frameCursors):new Map();
    const frames=[],nextCursors=new Map(),mirrored=new Set();let remainingMedia=2;
    // The site never receives a child's events. rrweb's cross-origin mode
    // uses postMessage to the parent page, which can expose private input.
    for(const frame of page.frames().slice(1,33)){
      if(frames.length>=8)break;
      const parent=frame.parentFrame();if(!parent)continue;
      let host;
      try{
        host=await bounded(frame.frameElement(),700);
        const hostId=await bounded(host.evaluate(element=>globalThis.__voyageMirror?.id(element)??-1),700);
        if(hostId<=0)continue;
        const frameId=this.frameId(frame),parentId=parent===page.mainFrame()?null:this.frameId(parent);
        if(parentId&&!nextCursors.has(parentId))continue;
        const cursor=frameCursors.get(frameId)||0;
        captureGuard();
        const child=await this.mirrorRead(frame,cursor,550000,generation,format,2500,guarded,producerGuard).catch(error=>{if(error.code==='operation_timeout')this.captureTaskUncertain=true;throw error;});
        if(child?.error)continue;
        if(format===null)this.inlineAssets(child.events,frame.url());
        const bytes=format===null?Buffer.from(JSON.stringify(child.events)):null;
        if(bytes&&bytes.length>650000)continue;
        let encoded=null;
        if(format==='css_chunks_v1'){encoded=this.prepareCssBatch(child,frame,frame.url(),generation,stamp,Math.min(cssCodec.limit-logical,cssCodec.limit-compactTotal));logical+=encoded.expanded_bytes;compactTotal+=encoded.value.total_bytes;if(logical>cssCodec.limit||compactTotal>cssCodec.limit)refuse('mirror_limit');}
        captureGuard();
        const frameVisuals=remainingMedia?await bounded(this.captureFrameVisuals(frame,frameId),1800).catch(error=>{if(error.code==='operation_timeout')this.captureTaskUncertain=true;return []; }):[];
        remainingMedia-=frameVisuals.length;
        frames.push({frame_id:frameId,parent_frame_id:parentId,host_node_id:hostId,
          ...(encoded?encoded.value:{encoding:'gzip',data_base64:gzipSync(bytes,{level:3}).toString('base64')}),cursor:child.cursor,reset:child.reset,visuals:frameVisuals});
        nextCursors.set(frameId,child.cursor);
        if(parent===page.mainFrame())mirrored.add(hostId);
      }catch(error){if(format==='css_chunks_v1'&&error.code==='mirror_limit')throw error;}finally{await host?.dispose().catch(()=>{});}
    }
    for(const id of this.frameVisuals.keys())if(!nextCursors.has(id))this.frameVisuals.delete(id);
    captureGuard();
    const visuals=await bounded(this.captureVisuals(page,mirrored),5000).catch(error=>{if(error.code==='operation_timeout')this.captureTaskUncertain=true;return this.visuals.filter(item=>!mirrored.has(item.id));});
    if(stamp!==this.epochs.capture||page!==this.page)refuse('capture_fenced');
    this.viewer(req.viewer);
    let encoded;
    if(format==='css_chunks_v1')encoded=mainEncoded;
    else{const bytes=Buffer.from(JSON.stringify(value.events));if(bytes.length>3000000)refuse('mirror_limit');encoded={encoding:'gzip',data_base64:gzipSync(bytes,{level:3}).toString('base64')};}
    const result={...encoded,cursor:value.cursor,reset:value.reset,latest:value.latest,visuals,frames};
    if(Buffer.byteLength(JSON.stringify(result))>2800000)refuse('mirror_limit');
    viewer.mirrorCursor=value.cursor;viewer.frameCursors=nextCursors;
    return result;
  }
  async fence(keepViewers=[]){
    this.advance('control','capture');this.downloads.clear();for(const f of this.fenceWaiters)f();this.fenceWaiters.clear();
    this.agentCursor=null;this.agentAction=null;for(const page of this.tabs.values())this.pageErrors.set(page,{console:0,page:0});
    await Promise.all([this.stopMirrors(),this.page&&!this.page.isClosed()?this.context.newCDPSession(this.page).then(async c=>{try{await c.send('Page.stopLoading');}finally{await c.detach();}}).catch(()=>{}):null]);
    // Acknowledgement means the old effect settled, not merely its raced reply.
    try{await bounded(Promise.allSettled([...this.effects]),5000);}catch{this.closing=true;await this.task?.close();await Promise.allSettled([...this.effects]);refuse('effect_quarantined');}
  }
  async agent(action){
    this.requireOpen();this.checkAgent();const stamp=this.epochs.control;
    let cancel;const fenced=new Promise((_,reject)=>{cancel=()=>reject(new Refusal('control_fenced'));this.fenceWaiters.add(cancel);});
    const effect=this.perform(action,stamp).catch(error=>{
      if(!['inspect','read','diagnostics','screenshot'].includes(action?.kind))throw error;
      // A failed read has no external effect to replay. Keep authority fences
      // strict, discard partial references, and let the caller observe again.
      this.guard(stamp);this.checkAgent();this.requireOpen();this.invalidate();
      if(error instanceof BeforeEffect)throw error;
      beforeEffect('observation_unavailable');
    });this.effects.add(effect);void effect.finally(()=>this.effects.delete(effect)).catch(()=>{});
    try{const value=await bounded(Promise.race([effect,fenced]));this.guard(stamp);this.checkAgent();return value;}finally{this.fenceWaiters.delete(cancel);await effect.catch(()=>{});}
  }
  async ref(id){
    text(id,128);const h=this.refs.get(id);if(!h)beforeEffect('stale_reference');
    const binding=this.refBindings.get(h);
    if(!binding||Date.now()-binding.at>60000||binding.frame.isDetached()||this.frameId(binding.frame)!==binding.frameId)beforeEffect('stale_reference');
    if(!await h.evaluate((e,version)=>{const state=globalThis.__voyageAgentObservation;state?.refresh();return e.isConnected&&state?.version===version;},binding.version))beforeEffect('stale_reference');
    if(digest(await h.evaluate(controlSignature))!==binding.signature)beforeEffect('stale_reference');
    return h;
  }
  async observedFrame(id){
    if(id===undefined||id===null)return this.page.mainFrame();
    const frame=this.agentFrames.get(identifier(id));
    if(!frame||frame.isDetached()||this.frameId(frame)!==id)beforeEffect('stale_frame');
    return frame;
  }
  async perform(a,stamp){
    if(!a||typeof a!=='object')refuse('invalid_action');const page=this.page,documentEpoch=this.epochs.document;
    const frameIdentities=new Map();
    const observedRef=async id=>{const h=await this.ref(id),binding=this.refBindings.get(h);frameIdentities.set(binding.frame,binding.frameId);return h;};
    const guard=()=>{this.guard(stamp);if(page!==this.page)refuse('tab_changed');if(documentEpoch!==this.epochs.document)refuse('document_changed');for(const [frame,id]of frameIdentities)if(frame.isDetached()||this.frameId(frame)!==id)refuse('frame_changed');};
    switch(a.kind){
      case 'navigate':origin(text(a.url,8192));await page.goto(a.url,{waitUntil:'domcontentloaded'});return null;
      case 'inspect':{
        const frame=await this.observedFrame(a.frame);frameIdentities.set(frame,this.frameId(frame));guard();this.invalidate();
        const offset=number(a.offset??0,0,100000),limit=number(a.limit??64,1,128),textOffset=number(a.text_offset??0,0,2000000);
        // A mutation invalidates all references from this observation. This is
        // deliberately conservative; agents must inspect after dynamic changes.
        const observed=await frame.evaluate(agentObservation,{offset,limit,textOffset});guard();
        const elements=[];let elementBytes=0,nextOffset=!observed.nodes_truncated&&offset+limit<observed.control_total?offset+limit:null;
        for(const i of observed.indices){
          const handle=await frame.evaluateHandle(i=>globalThis.__voyageAgentObservation?.controls[i]??null,i),h=handle.asElement();guard();if(!h){await handle.dispose();beforeEffect('stale_reference');}
          const info=await h.evaluate(e=>{
            const style=getComputedStyle(e),rect=e.getBoundingClientRect();
            if(e.type==='hidden'||!rect.width||!rect.height||style.visibility==='hidden'||style.visibility==='collapse'||e.closest('[inert]'))return null;
            const root=e.getRootNode();
            const label=(e.getAttribute('aria-labelledby')||'').split(/\s+/).map(id=>root.getElementById?.(id)?.textContent||'').join(' ').trim();
            return {tag:e.tagName.toLowerCase(),type:e.getAttribute('type')||null,role:e.getAttribute('role')||({BUTTON:'button',A:'link',SELECT:'combobox',TEXTAREA:'textbox'}[e.tagName])||(e.type==='checkbox'?'checkbox':e.type==='radio'?'radio':'textbox'),
              text:(e.getAttribute('aria-label')||label||Array.from(e.labels||[]).map(l=>l.innerText).join(' ')||e.innerText||e.getAttribute('placeholder')||e.getAttribute('name')||'').slice(0,256),
              checked:typeof e.checked==='boolean'?e.checked:null,selected:e.tagName==='SELECT'?[...e.selectedOptions].map(o=>o.label.slice(0,256)).slice(0,32):null,
              options:e.tagName==='SELECT'?[...e.options].slice(0,64).map(o=>({label:o.label.slice(0,256),value:o.value.slice(0,256),disabled:o.disabled})):null,
              options_truncated:e.tagName==='SELECT'&&e.options.length>64,
              disabled:e.matches(':disabled')||e.getAttribute('aria-disabled')==='true',readonly:e.readOnly===true};
          });guard();
          if(!info){await h.dispose();continue;}info.obscured=!await h.evaluate(exposed);guard();while(info.options?.length&&Buffer.byteLength(JSON.stringify(info))>16384){info.options.pop();info.options_truncated=true;}const size=Buffer.byteLength(JSON.stringify(info))+64;if(elementBytes+size>32768){await h.dispose();nextOffset=i;break;}elementBytes+=size;const ref=randomUUID();this.refs.set(ref,h);this.refBindings.set(h,{frame,frameId:this.frameId(frame),version:observed.version,at:Date.now(),signature:digest(await h.evaluate(controlSignature))});guard();elements.push({ref,...info});
        }
        const frames=[];for(const child of page.frames().slice(1,33)){const id=this.frameId(child);this.agentFrames.set(id,child);frames.push({frame:id,parent:child.parentFrame()===page.mainFrame()?null:this.frameId(child.parentFrame()),url:safeLocation(child.url()).slice(0,512)});}
        if(!await frame.evaluate(version=>{const state=globalThis.__voyageAgentObservation;state?.refresh();return state?.version===version;},observed.version))beforeEffect('observation_changed');guard();
        const title=(await page.title()).slice(0,256);guard();
        const location=new URL(frame.url());location.username='';location.password='';
        return {document:{url:location.href.slice(0,8192),title,frame:frame===page.mainFrame()?null:this.frameId(frame)},text:observed.text,text_total:observed.text_total,text_truncated:observed.text_truncated,elements,control_total:observed.control_total,controls_truncated:observed.controls_truncated,next_offset:nextOffset,frames,frames_truncated:page.frames().length>33,unsupported:['canvas and video require screenshot','closed shadow roots are not inspected',...(observed.controls_truncated?['control traversal limited to 100000 elements and 32 open shadow levels']:[]),...(observed.nodes_truncated?['rendered text and control references withheld because document traversal is incomplete']:[])],downloads:[...this.downloads].filter(([,d])=>d.owner==='agent').map(([id])=>id)};
      }
      case 'read':{
        const h=await observedRef(a.ref);guard();const offset=number(a.offset??0,0,2000000);
        const result=await h.evaluate((e,offset)=>{const content=e.innerText||e.textContent||'';return {tag:e.tagName.toLowerCase(),text:content.slice(offset,offset+16384),total:content.length,truncated:offset+16384<content.length};},offset);guard();return result;
      }
      case 'diagnostics':{
        // Page log text can contain secrets. Return content-free error categories.
        return {errors:this.pageErrors?.get(page)||{console:0,page:0},scope:'current tab since last control fence; log content withheld',dialog:!!this.dialog};
      }
      case 'history':{
        if(!['back','forward','reload'].includes(a.direction))beforeEffect('invalid_history');
        await page[a.direction==='back'?'goBack':a.direction==='forward'?'goForward':'reload']({waitUntil:'domcontentloaded',timeout:3000});return null;
      }
      case 'key':{
        const key=beforeText(a.key,128);if(!/^(?:(?:ControlOrMeta|Control|Alt|Shift|Meta)\+)*(?:[A-Za-z0-9]|Tab|Enter|Escape|Space|Backspace|Delete|ArrowUp|ArrowDown|ArrowLeft|ArrowRight|Home|End|PageUp|PageDown)$/.test(key))beforeEffect('invalid_key');
        const h=await observedRef(a.ref);guard();if(!await h.isVisible()||!await h.isEnabled())beforeEffect('element_unavailable');guard();
        await h.press(key,{timeout:3000});return null;
      }
      case 'select':case 'check':case 'double_click':case 'drag':{
        const h=await observedRef(a.ref);guard();if(!await h.isVisible()||!await h.isEnabled())beforeEffect('element_unavailable');guard();
        if(!await h.evaluate(exposed))beforeEffect('element_obscured');guard();
        if(a.kind==='select'){
          const value=beforeText(a.value,256);if(!await h.evaluate((e,value)=>e.tagName==='SELECT'&&[...e.options].some(o=>o.value===value&&!o.disabled),value))beforeEffect('unsupported_select');guard();await h.selectOption(value,{timeout:3000});
        }else if(a.kind==='check'){
          if(typeof a.checked!=='boolean'||!await h.evaluate(e=>e.tagName==='INPUT'&&['checkbox','radio'].includes(e.type)))beforeEffect('unsupported_check');if(!a.checked&&await h.evaluate(e=>e.type==='radio'))beforeEffect('unsupported_radio_uncheck');guard();await h.setChecked(a.checked,{timeout:3000});
        }else if(a.kind==='double_click')await h.dblclick({timeout:3000});
        else{
          const sourcePoint=await h.evaluate(exposed,true);guard();
          const target=await observedRef(a.target);guard();const targetPoint=await target.evaluate(exposed,true);guard();if(!await target.isVisible()||!sourcePoint||!targetPoint)beforeEffect('element_obscured');guard();
          const from=await h.boundingBox(),to=await target.boundingBox();guard();if(!from||!to)beforeEffect('element_hidden');
          await page.mouse.move(from.x+from.width*sourcePoint.x,from.y+from.height*sourcePoint.y);guard();await page.mouse.down();try{guard();await page.mouse.move(to.x+to.width*targetPoint.x,to.y+to.height*targetPoint.y,{steps:10});guard();}finally{await page.mouse.up();}
        }return null;
      }
      case 'click':case 'fill':{
        const h=await observedRef(a.ref);guard();const box=await h.boundingBox();guard();if(!box||!await h.isVisible())beforeEffect('element_hidden');guard();
        if(!await h.isEnabled())beforeEffect('element_disabled');guard();
        if(a.kind==='fill'&&!await h.isEditable())beforeEffect('element_not_editable');guard();
        const point=await h.evaluate(exposed,true);guard();if(!point)beforeEffect('element_obscured');guard();
        if(a.kind==='fill')text(a.text);
        this.agentCursor={x:box.x+box.width*point.x,y:box.y+box.height*point.y,width:this.config.width,height:this.config.height,at:Date.now()};
        // Playwright performs its own non-force actionable hit-test and chooses
        // a real content quad, rather than clicking the aggregate box center.
        await h.click({timeout:3000});
        if(a.kind==='click'){this.guard(stamp);if(page!==this.page)refuse('tab_changed');return null;}
        guard();
        if(a.kind==='fill'){await page.keyboard.press('ControlOrMeta+A');guard();await page.keyboard.insertText(a.text);}return null;
      }
      case 'scroll':await page.mouse.wheel(number(a.x,-16384,16384),number(a.y,-16384,16384));return null;
      case 'screenshot':{
        const limit=a.max_bytes===undefined?MAX_BYTES:number(a.max_bytes,0,MAX_BYTES);let bytes;
        for(const quality of [80,60,40,20]){bytes=await page.screenshot({type:'jpeg',quality,timeout:3000});guard();if(bytes.length<=limit)break;}
        return {mime_type:'image/jpeg',data_base64:bytes.toString('base64')};
      }
      case 'tabs':return this.tabAction(a,stamp);
      case 'upload':{
        const h=await observedRef(a.ref);guard();const name=text(a.name,255);if(!name||name!==path.basename(name)||name.includes('\\'))refuse('invalid_filename');const mimeType=text(a.mime_type,128);const raw=text(a.data_base64,Math.ceil(MAX_BYTES/3)*4);if(!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(raw))refuse('invalid_base64');const buffer=Buffer.from(raw,'base64');if(buffer.length>MAX_BYTES)refuse('upload_limit');await h.setInputFiles({name,mimeType,buffer},{timeout:3000});return null;
      }
      case 'download':{identifier(a.download_id);const d=this.downloads.get(a.download_id);if(!d||d.owner!=='agent')refuse('download_missing');this.downloads.delete(a.download_id);return d;}
      default:refuse('invalid_action');
    }
  }
  async tabAction(a,stamp){
    if(a.operation==='list')return {tabs:[...this.tabs.keys()],active:this.active};
    if(a.operation==='new'){if(this.tabs.size>=16)refuse('tab_limit');const p=await this.context.newPage();if(stamp!==this.epochs.control){await p.close();this.guard(stamp);}await this.select(this.idFor(p),stamp);return null;}
    identifier(a.tab);const p=this.tabs.get(a.tab);if(!p)refuse('tab_missing');
    if(a.operation==='select'){await this.select(a.tab,stamp);return null;}
    if(a.operation==='close'){if(this.tabs.size===1)refuse('last_tab');const active=this.active===a.tab;await p.close({runBeforeUnload:false});this.guard(stamp);if(active)await this.select(this.tabs.keys().next().value,stamp);return null;}
    refuse('invalid_tab_operation');
  }
  async claimInput(req){
    const viewer=this.viewer(req.viewer);this.requireOpen();
    if(this.mode!=='agent'||this.controller)refuse('controller_busy');
    if(!Number.isSafeInteger(req.seq)||req.seq!==viewer.seq+1)refuse('input_sequence');
    const action=req.action;
    const element=action?.kind==='element'&&['click','surface_click','wheel'].includes(action.action);
    if(!element&&!['history','navigate','tabs'].includes(action?.kind))refuse('invalid_claim');
    const page=this.page,document=this.epochs.document,tab=this.epochs.tab;
    let claimed=null;
    try{
      if(element){
        const id=number(action.node_id,1,Number.MAX_SAFE_INTEGER);
        let frame=page.mainFrame();
        if(action.frame_id!==undefined){
          const frameId=identifier(action.frame_id);
          frame=page.frames().find(item=>this.frameIds.get(item)===frameId);
          if(!frame||frame===page.mainFrame())beforeEffect('stale_frame');
        }
        const handle=await frame.evaluateHandle(node=>globalThis.__voyageMirror?.node(node)??null,id);
        claimed={handle,frame};
        if(!handle.asElement())beforeEffect('stale_reference');
      }
      this.checkAgent();
      if(page!==this.page||document!==this.epochs.document||tab!==this.epochs.tab)beforeEffect('stale_document');
      this.mode='human';this.controller=req.viewer;
      await this.fence([...this.viewers.keys()]);
      if(page!==this.page||document!==this.epochs.document||tab!==this.epochs.tab)beforeEffect('stale_document');
      if(claimed&&!await claimed.handle.evaluate(node=>node.isConnected).catch(()=>false))beforeEffect('stale_reference');
      const effect=this.input(req,claimed);
      this.effects.add(effect);
      try{return await effect;}finally{this.effects.delete(effect);}
    }finally{await claimed?.handle.dispose().catch(()=>{});}
  }
  async input(req,claimed=null){
    const viewer=this.viewer(req.viewer);if(this.controller!==req.viewer||this.mode==='agent')refuse('not_controller');this.requireOpen();
    if(!Number.isSafeInteger(req.seq)||req.seq!==viewer.seq+1)refuse('input_sequence');viewer.seq=req.seq;
    const a=req.action,stamp=this.epochs.control;if(!a)refuse('invalid_input');
    switch(a.kind){
      case 'dialog':if(typeof a.accept!=='boolean')refuse('invalid_input');if(a.text!=null)text(a.text);if(!this.dialog)refuse('dialog_missing');{const d=this.dialog;this.dialog=null;await (a.accept?d.accept(a.text??undefined):d.dismiss());}break;
      case 'element':{
        const id=number(a.node_id,1,Number.MAX_SAFE_INTEGER);
        if(!['click','surface_click','fill','select','wheel','upload'].includes(a.action))refuse('invalid_input');
        let frame=this.page.mainFrame();
        if(a.frame_id!==undefined){const frameId=identifier(a.frame_id);frame=this.page.frames().find(item=>this.frameIds.get(item)===frameId);if(!frame||frame===this.page.mainFrame())beforeEffect('stale_frame');}
        if(claimed&&claimed.frame!==frame)beforeEffect('stale_frame');
        const handle=claimed?.handle??await frame.evaluateHandle(node=>globalThis.__voyageMirror?.node(node)??null,id);
        try{
          const element=handle.asElement();if(!element)beforeEffect('stale_reference');
          if(!await element.isVisible())beforeEffect('element_hidden');
          if(a.frame_id!==undefined&&this.frameIds.get(frame)!==a.frame_id)beforeEffect('stale_frame');
          if(a.action==='surface_click'){
            const box=await element.boundingBox();if(!box)beforeEffect('element_hidden');
            await this.page.mouse.click(box.x+box.width*number(a.x,0,10000)/10000,
              box.y+box.height*number(a.y,0,10000)/10000,{button:a.button??'left'});
          }else if(a.action==='click'){
            if(!await element.isEnabled())beforeEffect('element_disabled');
            if(!await element.evaluate(exposed))beforeEffect('element_obscured');
            await element.click({button:a.button??'left',timeout:3000});
          }else if(a.action==='fill'){
            if(!await element.isEditable())beforeEffect('element_not_editable');
            await element.fill(text(a.text));
          }else if(a.action==='select'){
            await element.selectOption(text(a.value));
          }else if(a.action==='upload'){
            const name=text(a.name,255),mimeType=text(a.mime_type,128),raw=text(a.data_base64,Math.ceil(MAX_BYTES/3)*4);
            if(!name||name!==path.basename(name)||name.includes('\\'))refuse('invalid_filename');
            if(!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(raw))refuse('invalid_base64');
            const buffer=Buffer.from(raw,'base64');if(buffer.length>MAX_BYTES)refuse('upload_limit');
            await element.setInputFiles({name,mimeType,buffer},{timeout:3000});
          }else{
            const box=await element.boundingBox();if(!box)beforeEffect('element_hidden');
            await this.page.mouse.move(box.x+box.width/2,box.y+box.height/2);
            this.guard(stamp);
            await this.page.mouse.wheel(number(a.x,-16384,16384),number(a.y,-16384,16384));
          }
        }finally{await handle.dispose().catch(()=>{});}
        break;
      }
      case 'key':if(!['down','up'].includes(a.type)||!text(a.key,128)||/[\x00-\x1f]/.test(a.key))refuse('invalid_input');await this.page.keyboard[a.type](a.key);break;
      case 'text':await this.page.keyboard.insertText(text(a.text));break;
      case 'scroll':await this.page.mouse.wheel(number(a.x,-16384,16384),number(a.y,-16384,16384));break;
      case 'resize':{
        const width=number(a.width,320,3840),height=number(a.height,240,2160);
        this.advance('viewport','capture');
        await this.page.setViewportSize({width,height});this.guard(stamp);
        this.config.width=width;this.config.height=height;
        break;
      }
      case 'history':{
        if(!['back','forward','reload','stop'].includes(a.direction))refuse('invalid_history');
        if(a.direction==='back')await this.page.goBack({waitUntil:'domcontentloaded'});
        else if(a.direction==='forward')await this.page.goForward({waitUntil:'domcontentloaded'});
        else if(a.direction==='reload')await this.page.reload({waitUntil:'domcontentloaded'});
        else {const c=await this.context.newCDPSession(this.page);try{await c.send('Page.stopLoading');}finally{await c.detach();}this.metadata.set(this.active,{...this.metadata.get(this.active),loading:false});}
        break;
      }
      case 'navigate':case 'tabs':await bounded(this.perform(a,stamp));break;
      case 'download':{
        identifier(a.download_id);const d=this.downloads.get(a.download_id);
        if(!d||d.owner!==req.viewer)refuse('download_missing');
        this.downloads.delete(a.download_id);return {name:d.name,data_base64:d.data_base64,mime_type:'application/octet-stream'};
      }
      default:refuse('invalid_input');
    }
    this.guard(stamp);return null;
  }
  async close(){
    this.closing=true;
    this.assetEpoch++;
    try{await this.fence();}catch{}
    const results=await Promise.allSettled([this.task?.close(),this.proxy?.close()]);
    if(results.some(r=>r.status==='rejected'))refuse('cleanup_failed');
    this.task=null;this.context=null;this.page=null;this.active=null;this.proxy=null;this.tabs.clear();this.metadata.clear();this.viewers.clear();this.downloads.clear();this.assets.clear();this.assetBytes=0;this.closing=false;this.recorderStopObserved=false;await this.publishCaptureObservation();
    if(results.some(r=>r.status==='rejected'))refuse('cleanup_failed');return null;
  }
  async releaseLock(){if(this.lock){await this.lock.close();this.lock=null;await fs.unlink(this.lockPath);}}
  async dispose(){await this.close();await this.releaseLock();}
}

export function stdio(){
  const worker=new Worker();let buffer=Buffer.alloc(0),ending=false,outstanding=0;
  const finish=async()=>{if(ending)return;ending=true;worker.disconnected=true;worker.closing=true;process.stdin.pause();const deadline=setTimeout(()=>process.exit(1),8000);try{await worker.fence();await worker.admission.catch(()=>{});await Promise.all([worker.ordinary,worker.urgent]);await worker.dispose();}catch{process.exitCode=1;}finally{clearTimeout(deadline);}};
  const write=reply=>{if(!process.stdout.write(JSON.stringify(reply)+'\n'))process.stdin.pause();};
  process.stdout.on('drain',()=>{if(!ending)process.stdin.resume();});
  process.stdout.on('error',()=>{void finish();});
  process.stdin.on('data',chunk=>{
    if(ending)return;buffer=Buffer.concat([buffer,chunk]);
    while(!ending){const at=buffer.indexOf(10);if(at<0)break;if(at>3*1024*1024){void finish();break;}const line=buffer.subarray(0,at);buffer=buffer.subarray(at+1);
      let req;try{req=JSON.parse(line.toString('utf8'));}catch{write({id:null,ok:false,error:{code:'invalid_json'}});continue;}
      if(++outstanding>64){void finish();break;}
      void worker.request(req).then(reply=>{write(reply);if(req.op==='shutdown'&&reply.ok)void finish();}).finally(()=>{outstanding--;});
    }
    if(buffer.length>3*1024*1024)void finish();
  });
  process.stdin.on('end',()=>{void finish();});process.stdin.on('error',()=>{void finish();});
  for(const signal of ['SIGTERM','SIGINT'])process.on(signal,()=>{void finish();});
  process.on('unhandledRejection',()=>{process.exitCode=1;void finish();});
  return worker;
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url))stdio();
