import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';
import { chromium } from 'playwright-core';
import { Journal, UUID, privateDir } from './journal.mjs';
import { Refusal, refuse, digest, origin, networkProxy } from './security.mjs';
import { encoderRuntime, LatestFrameQueue } from './media-next.mjs';

class BeforeEffect extends Refusal {}
const beforeEffect = code => { throw new BeforeEffect(code); };
const exposed = e => {const r=e.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2,hit=document.elementFromPoint(x,y);return !!hit&&(hit===e||e.contains(hit));};

const MAX_TEXT=16384, MAX_BYTES=2*1024*1024;
const text=(v,max=MAX_TEXT)=>{if(typeof v!=='string'||Buffer.byteLength(v)>max)refuse('invalid_text');return v;};
const number=(v,min,max)=>{if(!Number.isInteger(v)||v<min||v>max)refuse('invalid_number');return v;};
const identifier=v=>{if(typeof v!=='string'||!UUID.test(v))refuse('invalid_id');return v;};
const bounded=async(p,ms=12000)=>{let timer;try{return await Promise.race([p,new Promise((_,r)=>{timer=setTimeout(()=>r(new Refusal('operation_timeout')),ms);})]);}finally{clearTimeout(timer);}};

export class Worker {
  constructor(){
    this.browser=randomUUID();this.epochs={tab:1,document:1,viewport:1,control:1,capture:1};
    this.mode='agent';this.controller=null;this.viewers=new Map();this.tabs=new Map();this.refs=new Map();this.downloads=new Map();
    this.ordinary=Promise.resolve();this.urgent=Promise.resolve();this.admission=Promise.resolve();this.ephemeral=new Map();this.pending=0;this.fenceWaiters=new Set();this.closing=false;this.disconnected=false;this.effects=new Set();this.metadata=new Map();this.agentActive=0;this.agentAction=null;this.agentCursor=null;
  }
  status(){return {viewport:{width:this.config?.width,height:this.config?.height},agent_action:this.mode==='agent'?this.agentAction:null,agent_cursor:this.mode==='agent'?this.agentCursor:null,agent_active:this.agentActive>0,page:this.metadata.get(this.active)||null,tab_details:[...this.tabs.keys()].map(id=>({id,...(this.metadata.get(id)||{})})),dialog:this.dialog?{type:this.dialog.type(),message:this.dialog.message().slice(0,1024)}:null,browser:this.browser,epochs:{...this.epochs},mode:this.mode,controller:this.controller,tabs:[...this.tabs.keys()],tab:this.active||null,viewers:[...this.viewers.keys()],open:!!this.task};}
  exact(req){const epochs={...req.epochs};if(['join','offer','answer','disconnect'].includes(req.op)){epochs.document=this.epochs.document;epochs.viewport=this.epochs.viewport;}if(req.browser!==this.browser||digest(epochs)!==digest(this.epochs))refuse('stale_binding');}
  invalidate(){for(const h of this.refs.values())void h.dispose().catch(()=>{});this.refs.clear();}
  advance(...keys){for(const k of keys)this.epochs[k]++;this.invalidate();}
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
          const ephemeral=['input','offer','answer','status','receipt'].includes(req.op);
          const old=ephemeral?this.ephemeral.get(req.id):this.journal.previous(req);
          if(old){if(old.digest!==digest(req))refuse('id_conflict');scheduled=Promise.resolve({receipt:old,content_withheld:true});return;}
          const r=ephemeral?{id:req.id,digest:digest(req),state:'dispatched',ephemeral:true}:await this.journal.begin(req);
          if(ephemeral){this.ephemeral.set(req.id,r);if(this.ephemeral.size>2048)this.ephemeral.delete(this.ephemeral.keys().next().value);}
          const urgent=['control','disconnect','close','shutdown','policy','status'].includes(req.op)||(req.op==='input'&&(req.action?.kind==='dialog'||(req.action?.kind==='history'&&req.action.direction==='stop')));
          const execute=()=>this.execute(req,r);
          const lane=urgent?'urgent':'ordinary';scheduled=this[lane].then(execute);this[lane]=scheduled.catch(()=>{});
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
    const servers=config.ice_servers??[];
    if(!Array.isArray(servers)||servers.length>4||typeof (config.relay_only??false)!=='boolean')refuse('invalid_ice');
    for(const s of servers){if(!Array.isArray(s.urls)||!s.urls.length||s.urls.length>4||s.urls.some(u=>typeof u!=='string'||u.length>2048||!/^turns?:[^\s/@]+(?::\d+)?(?:\?transport=(?:tcp|udp))?$/.test(u)))refuse('invalid_ice');text(s.username,1024);text(s.credential,4096);}
    if(config.relay_only&&!servers.length)refuse('relay_requires_turn');this.config.ice_servers=servers;
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
      await this.refreshMetadata();await this.finishReceipt(r,'completed');return {status:this.status(),value};
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
      case 'join':identifier(req.viewer);if(this.viewers.has(req.viewer))refuse('viewer_exists');if(this.viewers.size>=4)refuse('viewer_limit');if(this.mode==='private'&&this.controller!==req.viewer)refuse('private');this.viewers.set(req.viewer,{seq:0,signalSeq:0});return null;
      case 'disconnect':{
        identifier(req.viewer);this.viewers.delete(req.viewer);
        if(this.controller===req.viewer){if(this.mode!=='private'){this.mode='agent';this.controller=null;}await this.fence();}
        else await this.encoder?.evaluate(v=>encoder.disconnect(v),req.viewer);
        if(!this.viewers.size)await this.stopCapture();return null;
      }
      case 'control':{
        this.viewer(req.viewer);if(!['agent','human','private'].includes(req.mode))refuse('invalid_mode');
        if(this.controller&&this.controller!==req.viewer)refuse('controller_busy');
        if(req.mode==='agent'&&this.controller!==req.viewer)refuse('not_controller');
        const capturing=!!this.cdp;
        this.mode=req.mode;this.controller=req.mode==='agent'?null:req.viewer;
        if(req.mode==='private')for(const id of this.viewers.keys())if(id!==req.viewer)this.viewers.delete(id);
        await this.fence([...this.viewers.keys()]);
        if(capturing&&this.viewers.size&&this.task&&!this.page?.isClosed())await this.startCapture();
        return null;
      }
      case 'offer':{
        this.signalSequence(req);this.requireOpen();await this.startCapture();
        const stamp=this.epochs.capture;
        const offer=await bounded(this.encoder.evaluate(a=>encoder.offer(a),{viewer:req.viewer,iceServers:this.config.ice_servers,relayOnly:!!this.config.relay_only}));
        if(stamp!==this.epochs.capture)refuse('capture_fenced');return offer;
      }
      case 'answer':this.signalSequence(req);this.requireOpen();if(req.description?.type!=='answer')refuse('invalid_sdp');text(req.description.sdp,65536);await bounded(this.encoder.evaluate(a=>encoder.answer(a),{viewer:req.viewer,description:req.description}));return null;
      case 'input':{const effect=this.input(req);this.effects.add(effect);try{return await effect;}finally{this.effects.delete(effect);}}
      case 'agent':this.agentActive++;this.agentAction=['inspect','navigate','click','fill','scroll','tabs','screenshot','upload','download'].includes(req.action?.kind)?req.action.kind:null;try{return await this.agent(req.action);}finally{this.agentActive--;this.agentAction=null;}
      default:refuse('unknown_operation');
    }
  }
  signalSequence(req){const v=this.viewer(req.viewer);if(!Number.isSafeInteger(req.signal_seq)||req.signal_seq!==v.signalSeq+1)refuse('signal_sequence');v.signalSeq=req.signal_seq;}
  viewer(id){identifier(id);if(!this.viewers.has(id))refuse('viewer_missing');if(this.mode==='private'&&this.controller!==id)refuse('private');return this.viewers.get(id);}
  requireOpen(){if(!this.task||!this.page||this.page.isClosed())refuse('browser_closed');}
  async open(){
    if(this.task)return null;
    this.proxy=await networkProxy(this.allowed,this.publicWeb);
    const base={executablePath:this.config.executable,headless:true,chromiumSandbox:true,timeout:15000,args:['--disable-background-networking','--disable-component-update','--disable-sync','--disable-quic','--force-webrtc-ip-handling-policy=disable_non_proxied_udp','--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1']};
    try{
      this.task=await chromium.launch({...base,proxy:{server:this.proxy.server,username:this.proxy.username,password:this.proxy.password,bypass:'<-loopback>'}});
      if(this.disconnected)refuse('parent_disconnected');
      this.context=await this.task.newContext({viewport:{width:this.config.width,height:this.config.height},acceptDownloads:true,serviceWorkers:'block'});
      this.context.setDefaultTimeout(5000);this.context.setDefaultNavigationTimeout(10000);
      await this.context.route('**/*',async route=>{
        try{const url=route.request().url();if(url==='about:blank')return route.continue();const o=origin(url);if(!this.publicWeb&&!this.allowed.has(o))return route.abort();return route.continue();}catch{return route.abort().catch(()=>{});}
      });
      // WebSockets use Chromium's authenticated proxy, including DNS-pinned
      // CONNECT checks; never forward via a Node-side WebSocket client.
      await this.context.addInitScript(()=>{Object.defineProperty(globalThis,'RTCPeerConnection',{value:undefined,configurable:false});Object.defineProperty(globalThis,'webkitRTCPeerConnection',{value:undefined,configurable:false});});
      this.context.on('page',p=>this.registerPage(p));
      this.encoding=await chromium.launch({...base,args:[...base.args.filter(a=>!a.startsWith('--force-webrtc')), '--disable-features=WebRtcHideLocalIpsWithMdns']});
      if(this.disconnected)refuse('parent_disconnected');
      this.encoder=await this.encoding.newPage({viewport:{width:1280,height:720}});
      await this.encoder.route('**/*',route=>route.abort());
      await this.encoder.evaluate(encoderRuntime);
      await this.encoder.evaluate(g=>encoder.fence({generation:g}),this.epochs.capture);
      this.frames=new LatestFrameQueue(frame=>this.encoder.evaluate(f=>encoder.frame(f),frame));this.frames.generation=this.epochs.capture;
      const page=await this.context.newPage();await this.select(this.idFor(page));
      return null;
    }catch(e){await this.close();throw e;}
  }
  idFor(page){for(const [id,p]of this.tabs)if(p===page)return id;}
  registerPage(page){
    if(this.tabs.size>=16){void page.close();return;}
    const id=randomUUID();this.tabs.set(id,page);
    page.on('request',request=>{if(request.isNavigationRequest()&&request.frame()===page.mainFrame())this.metadata.set(id,{...this.metadata.get(id),loading:true});});
    page.on('load',()=>{this.metadata.set(id,{...this.metadata.get(id),loading:false});});
    page.on('dialog',dialog=>{if(page===this.page)this.dialog=dialog;else void dialog.dismiss().catch(()=>{});});
    page.on('framenavigated',frame=>{if(page===this.page&&frame===page.mainFrame()){this.advance('document');this.dialog=null;}});
    page.on('close',()=>{this.metadata.delete(id);this.tabs.delete(id);if(this.page===page){this.page=null;this.active=null;this.advance('tab','document');void this.stopCapture();}});
    page.on('download',download=>{void this.recordDownload(download);});
  }
  async recordDownload(download){
    const stamp=this.epochs.control;
    try{const file=await download.path();if(!file)return;const s=await fs.stat(file);if(s.size<=MAX_BYTES&&this.mode==='agent'&&stamp===this.epochs.control&&this.downloads.size<8){const data=await fs.readFile(file);if(this.mode==='agent'&&stamp===this.epochs.control)this.downloads.set(randomUUID(),{name:download.suggestedFilename(),data_base64:data.toString('base64')});}}catch{}finally{await download.delete().catch(()=>{});}
  }
  async select(id,stamp=this.epochs.control){this.guard(stamp);const page=this.tabs.get(id);if(!page)refuse('tab_missing');await this.stopCapture();this.guard(stamp);this.page=page;this.active=id;this.dialog=null;this.advance('tab','document','capture');await page.setViewportSize({width:this.config.width,height:this.config.height});this.guard(stamp);await page.bringToFront();this.guard(stamp);if(this.encoder)await this.encoder.evaluate(g=>encoder.fence({generation:g}),this.epochs.capture);this.guard(stamp);if(this.frames)await this.frames.fence(this.epochs.capture);this.guard(stamp);}
  async stopCapture(){const cdp=this.cdp;this.cdp=null;if(cdp){await cdp.send('Page.stopScreencast').catch(()=>{});await cdp.detach().catch(()=>{});}}
  async startCapture(){
    if(this.cdp)return;this.requireOpen();const cdp=await this.context.newCDPSession(this.page);this.cdp=cdp;
    cdp.on('Page.screencastFrame',event=>{
      const delivered=this.cdp===cdp&&this.viewers.size
        ?this.frames.push({data:event.data,width:Math.round(event.metadata.deviceWidth),height:Math.round(event.metadata.deviceHeight)})
        :Promise.resolve();
      void delivered.finally(()=>cdp.send('Page.screencastFrameAck',{sessionId:event.sessionId}).catch(()=>{}));
    });
    await cdp.send('Page.startScreencast',{format:'jpeg',quality:80,maxWidth:1920,maxHeight:1080,everyNthFrame:1});
  }
  async fence(keepViewers=[]){
    this.advance('control','capture');this.downloads.clear();for(const f of this.fenceWaiters)f();this.fenceWaiters.clear();
    this.agentCursor=null;this.agentAction=null;
    const generation=this.epochs.capture;
    // Invalidate before any await. Reset encoder before draining old JPEG work.
    if(this.frames){this.frames.generation=generation;this.frames.pending=null;}
    await Promise.all([this.stopCapture(),this.encoder?.evaluate(args=>encoder.fence(args),{generation,keepViewers}),this.page&&!this.page.isClosed()?this.context.newCDPSession(this.page).then(async c=>{try{await c.send('Page.stopLoading');}finally{await c.detach();}}).catch(()=>{}):null]);
    if(this.frames)await this.frames.fence(generation);
    // Acknowledgement means the old effect settled, not merely its raced reply.
    try{await bounded(Promise.allSettled([...this.effects]),5000);}catch{this.closing=true;await this.task?.close();await Promise.allSettled([...this.effects]);refuse('effect_quarantined');}
  }
  async agent(action){
    this.requireOpen();this.checkAgent();const stamp=this.epochs.control;
    let cancel;const fenced=new Promise((_,reject)=>{cancel=()=>reject(new Refusal('control_fenced'));this.fenceWaiters.add(cancel);});
    const effect=this.perform(action,stamp).catch(error=>{
      if(!['inspect','screenshot'].includes(action?.kind))throw error;
      // A failed read has no external effect to replay. Keep authority fences
      // strict, discard partial references, and let the caller observe again.
      this.guard(stamp);this.checkAgent();this.requireOpen();this.invalidate();
      beforeEffect('observation_unavailable');
    });this.effects.add(effect);void effect.finally(()=>this.effects.delete(effect)).catch(()=>{});
    try{const value=await bounded(Promise.race([effect,fenced]));this.guard(stamp);this.checkAgent();return value;}finally{this.fenceWaiters.delete(cancel);await effect.catch(()=>{});}
  }
  async ref(id){text(id,128);const h=this.refs.get(id);if(!h)beforeEffect('stale_reference');return h;}
  async perform(a,stamp){
    if(!a||typeof a!=='object')refuse('invalid_action');const page=this.page,documentEpoch=this.epochs.document;
    const guard=()=>{this.guard(stamp);if(page!==this.page)refuse('tab_changed');if(documentEpoch!==this.epochs.document)refuse('document_changed');};
    switch(a.kind){
      case 'navigate':origin(text(a.url,8192));await page.goto(a.url,{waitUntil:'domcontentloaded'});return null;
      case 'inspect':{
        this.invalidate();
        const body=await page.locator('body').innerText({timeout:3000});guard();
        const handles=await page.$$('a,button,input,textarea,select,[role="button"],[contenteditable="true"]');guard();const elements=[];
        for(const h of handles){
          if(elements.length>=128){await h.dispose();continue;}
          const info=await h.evaluate(e=>{
            const style=getComputedStyle(e),rect=e.getBoundingClientRect();
            if(e.type==='hidden'||!rect.width||!rect.height||style.visibility==='hidden'||style.visibility==='collapse'||e.closest('[inert]'))return null;
            const label=(e.getAttribute('aria-labelledby')||'').split(/\s+/).map(id=>document.getElementById(id)?.textContent||'').join(' ').trim();
            return {tag:e.tagName.toLowerCase(),type:e.getAttribute('type')||null,
              text:(e.getAttribute('aria-label')||label||Array.from(e.labels||[]).map(l=>l.innerText).join(' ')||e.innerText||e.getAttribute('placeholder')||e.getAttribute('name')||'').slice(0,256),
              disabled:e.matches(':disabled')||e.getAttribute('aria-disabled')==='true',readonly:e.readOnly===true,obscured:!((hit=>hit&&(hit===e||e.contains(hit)))(document.elementFromPoint(rect.x+rect.width/2,rect.y+rect.height/2)))};
          });guard();
          if(!info){await h.dispose();continue;}const ref=randomUUID();this.refs.set(ref,h);elements.push({ref,...info});
        }
        const title=(await page.title()).slice(0,256);guard();
        const location=new URL(page.url());location.username='';location.password='';
        // Document identity sorts before large control lists in Rust's JSON
        // projection, so bounded model previews retain the navigation result.
        return {document:{url:location.href.slice(0,8192),title},text:body.slice(0,MAX_TEXT),elements,downloads:[...this.downloads.keys()]};
      }
      case 'click':case 'fill':{
        const h=await this.ref(a.ref);guard();const box=await h.boundingBox();guard();if(!box||!await h.isVisible())beforeEffect('element_hidden');guard();
        if(!await h.isEnabled())beforeEffect('element_disabled');guard();
        if(a.kind==='fill'&&!await h.isEditable())beforeEffect('element_not_editable');guard();
        if(!await h.evaluate(exposed))beforeEffect('element_obscured');guard();
        if(a.kind==='fill')text(a.text);
        this.agentCursor={x:box.x+box.width/2,y:box.y+box.height/2,width:this.config.width,height:this.config.height,at:Date.now()};
        await page.mouse.click(box.x+box.width/2,box.y+box.height/2,{timeout:3000});
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
        const h=await this.ref(a.ref);guard();const name=text(a.name,255);if(!name||name!==path.basename(name)||name.includes('\\'))refuse('invalid_filename');const mimeType=text(a.mime_type,128);const raw=text(a.data_base64,Math.ceil(MAX_BYTES/3)*4);if(!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(raw))refuse('invalid_base64');const buffer=Buffer.from(raw,'base64');if(buffer.length>MAX_BYTES)refuse('upload_limit');await h.setInputFiles({name,mimeType,buffer},{timeout:3000});return null;
      }
      case 'download':{identifier(a.download_id);const d=this.downloads.get(a.download_id);if(!d)refuse('download_missing');this.downloads.delete(a.download_id);return d;}
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
  async input(req){
    const viewer=this.viewer(req.viewer);if(this.controller!==req.viewer||this.mode==='agent')refuse('not_controller');this.requireOpen();
    if(!Number.isSafeInteger(req.seq)||req.seq!==viewer.seq+1)refuse('input_sequence');viewer.seq=req.seq;
    const a=req.action,stamp=this.epochs.control;if(!a)refuse('invalid_input');
    switch(a.kind){
      case 'dialog':if(typeof a.accept!=='boolean')refuse('invalid_input');if(a.text!=null)text(a.text);if(!this.dialog)refuse('dialog_missing');{const d=this.dialog;this.dialog=null;await (a.accept?d.accept(a.text??undefined):d.dismiss());}break;
      case 'pointer':number(a.x,0,this.config.width);number(a.y,0,this.config.height);if(!['move','down','up'].includes(a.type)||!['left','middle','right'].includes(a.button??'left'))refuse('invalid_input');await this.page.mouse.move(a.x,a.y);this.guard(stamp);if(a.type!=='move')await this.page.mouse[a.type]({button:a.button??'left'});break;
      case 'key':if(!['down','up'].includes(a.type)||!text(a.key,128)||/[\x00-\x1f]/.test(a.key))refuse('invalid_input');await this.page.keyboard[a.type](a.key);break;
      case 'text':await this.page.keyboard.insertText(text(a.text));break;
      case 'scroll':await this.page.mouse.wheel(number(a.x,-16384,16384),number(a.y,-16384,16384));break;
      case 'resize':{
        const width=number(a.width,320,3840),height=number(a.height,240,2160);
        const capturing=!!this.cdp;
        this.advance('viewport','capture');
        await this.stopCapture();
        if(this.frames)await this.frames.fence(this.epochs.capture);
        await this.encoder?.evaluate(args=>encoder.fence(args),{generation:this.epochs.capture,keepViewers:[...this.viewers.keys()]});
        await this.page.setViewportSize({width,height});this.guard(stamp);
        this.config.width=width;this.config.height=height;
        if(capturing)await this.startCapture();
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
      default:refuse('invalid_input');
    }
    this.guard(stamp);return null;
  }
  async close(){
    this.closing=true;
    try{await this.fence();}catch{}
    const results=await Promise.allSettled([this.task?.close(),this.encoding?.close(),this.proxy?.close()]);
    if(results.some(r=>r.status==='rejected'))refuse('cleanup_failed');
    this.task=null;this.encoding=null;this.encoder=null;this.context=null;this.page=null;this.active=null;this.proxy=null;this.frames=null;this.tabs.clear();this.metadata.clear();this.viewers.clear();this.downloads.clear();this.closing=false;
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
