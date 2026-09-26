const NIL = '00000000-0000-0000-0000-000000000000';
const bytes = value => new TextEncoder().encode(value).length;
const bindingKey = status => JSON.stringify(status?.binding);
const pageKey = status => JSON.stringify([
    status?.binding?.incarnation, status?.binding?.browser_id,
    status?.binding?.attachment_id, status?.binding?.tab_id,
    status?.binding?.document_epoch, status?.binding?.controller_epoch,
    status?.binding?.capture_epoch,
]);
const staleBinding=(current,next)=>!!current&&!!next&&current.browser_id===next.browser_id
    && ['document_epoch','viewport_epoch','controller_epoch','capture_epoch'].some(name=>(current[name]||0)>(next[name]||0));
const safe = (value, limit = 512) => typeof value === 'string'
    ? value.replace(/[\x00-\x1f\x7f-\x9f\u202a-\u202e\u2066-\u2069]/g,'').slice(0,limit) : '';
const replayPolicy="default-src 'none'; img-src data: blob:; font-src data: blob:; media-src data: blob:; style-src 'unsafe-inline'; frame-src data: blob:; form-action 'none'; base-uri 'none'";

function fenceSnapshot(event) {
    if(event?.type!==2)return event;
    const html=event.data?.node?.childNodes?.find(node=>node.tagName==='html');
    const head=html?.childNodes?.find(node=>node.tagName==='head');
    if(!Array.isArray(head?.childNodes))throw Error('invalid_full_snapshot');
    // rrweb rebuilds the iframe document. Place the policy before any captured
    // resource node so the rebuilt document never starts a site fetch from Helm.
    head.childNodes.unshift({type:2,tagName:'meta',attributes:{'http-equiv':'Content-Security-Policy',content:replayPolicy},childNodes:[],id:Number.MAX_SAFE_INTEGER-1});
    return event;
}

async function decodeMirror(value) {
    if (value?.encoding !== 'gzip' || typeof value.data_base64 !== 'string' || value.data_base64.length > 4000000)
        throw Error('invalid_mirror');
    const raw = Uint8Array.from(atob(value.data_base64), char => char.charCodeAt(0));
    const stream = new Blob([raw]).stream().pipeThrough(new DecompressionStream('gzip'));
    const content = await new Response(stream).text();
    if (bytes(content) > 3000000) throw Error('mirror_limit');
    const events = JSON.parse(content);
    if (!Array.isArray(events) || events.length > 1024) throw Error('invalid_mirror');
    return events;
}

// A mount owns one authenticated attachment and one script-free replay iframe.
// Read-only mirror cursors may be repeated; page actions never are.
export class BrowserConnection {
    constructor({transport, context, mirror, frameLayer, changed = () => {}, onReplay = () => {}, onVisuals = () => {},
        uuid = () => crypto.randomUUID(), timeout = 25000}) {
        Object.assign(this,{transport,context,mirror,frameLayer,changed,onReplay,onVisuals,uuid,timeout});
        this.status=null;this.phase='idle';this.issue=null;this.sequence=0;
        this.queue=[];this.sending=false;this.urgentPromise=null;this.busy=false;this.closed=false;
        this.epoch=0;this.cursor=0;this.streaming=false;this.polling=false;this.replayer=null;this.frames=new Map();
    }
    emit(){this.changed(this);}
    operation(action,fields={}){return {action,command_id:this.uuid(),binding:{...this.status.binding},...fields};}
    get attached(){return !!this.status?.binding && this.status.binding.attachment_id!==NIL;}
    get controls(){return this.attached && ['human','private'].includes(this.status?.mode)
        && this.status.controller===this.status.binding.attachment_id;}
    get canInput(){return this.controls&&this.streaming&&!this.busy&&!this.issue&&!this.closed;}
    discardQueued(reason){this.queue.splice(0).forEach(item=>item.reject?.(Error(reason)));}
    clearMirror(){
        this.cursor=0;this.streaming=false;
        this.replayer?.destroy();this.replayer=null;
        for(const frame of this.frames.values())frame.player.destroy();
        this.frames.clear();this.frameLayer?.replaceChildren();
        this.mirror?.replaceChildren();
        this.onVisuals([],null);
    }
    newPlayer(root,frameId=null){
        const library=globalThis.rrweb;
        if(!library?.Replayer)throw Error('replayer_unavailable');
        const player=new library.Replayer([],{root,liveMode:true,mouseTail:false,
            showWarning:false,UNSAFE_replayCanvas:false,loadTimeout:500});
        const replayDocument=player.iframe.contentDocument;
        const policy=replayDocument.createElement('meta');
        policy.httpEquiv='Content-Security-Policy';policy.content=replayPolicy;
        replayDocument.head.append(policy);
        player.iframe.referrerPolicy='no-referrer';
        player.enableInteract();
        player.on('fullsnapshot-rebuilded',()=>{
            if(frameId===null){
                if(this.replayer!==player)return;
                this.streaming=true;this.phase='live';this.issue=null;this.emit();
                this.onVisuals(this.latestVisuals||[],player);
            }else if(this.frames.get(frameId)?.player!==player)return;
            this.onReplay(player,this,frameId);
            if(frameId!==null){const frame=this.frames.get(frameId);this.paintFrameVisuals(frame,frame.latestVisuals||[]);}
        });
        player.startLive(Date.now()-120);
        return player;
    }
    framePosition(item,parent){
        const target=parent.player.getMirror().getNode(item.host_node_id);
        if(!target?.getBoundingClientRect)return null;
        const rect=target.getBoundingClientRect();
        let left=rect.left+target.clientLeft,top=rect.top+target.clientTop;
        let owner=target.ownerDocument;
        while(owner!==parent.player.iframe.contentDocument){
            const host=owner?.defaultView?.frameElement;
            if(!host)return null;
            const outer=host.getBoundingClientRect();left+=outer.left+host.clientLeft;top+=outer.top+host.clientTop;
            owner=host.ownerDocument;
        }
        return {left,top,
            width:target.clientWidth||rect.width,height:target.clientHeight||rect.height};
    }
    paintFrameVisuals(frame,items){
        if(!frame?.visualLayer)return;
        const present=new Set();
        for(const item of items){
            if(!Number.isSafeInteger(item.id)||typeof item.data_base64!=='string'||item.data_base64.length>270000)continue;
            const position=this.framePosition({host_node_id:item.id},{player:frame.player});
            if(!position)continue;
            present.add(item.id);
            let picture=frame.visualNodes.get(item.id);
            if(!picture){picture=document.createElement('img');picture.className='browser-next-frame-visual';picture.alt='';frame.visualLayer.append(picture);frame.visualNodes.set(item.id,picture);}
            if(picture.dataset.version!==String(item.version)){
                picture.src=`data:image/jpeg;base64,${item.data_base64}`;
                picture.dataset.version=String(item.version);
            }
            picture.style.left=`${position.left}px`;picture.style.top=`${position.top}px`;
            picture.style.width=`${Math.max(0,position.width)}px`;picture.style.height=`${Math.max(0,position.height)}px`;
        }
        for(const [id,picture] of frame.visualNodes)if(!present.has(id)){picture.remove();frame.visualNodes.delete(id);}
    }
    async updateFrames(items){
        if(!this.frameLayer)return;
        if(!Array.isArray(items)||items.length>8)throw Error('invalid_frames');
        const present=new Set();
        for(const item of items){
            if(typeof item.frame_id!=='string'||present.has(item.frame_id)||!Number.isSafeInteger(item.host_node_id)||item.host_node_id<=0)throw Error('invalid_frame');
            present.add(item.frame_id);
            const parent=item.parent_frame_id?this.frames.get(item.parent_frame_id):{player:this.replayer,root:this.frameLayer};
            if(!parent?.player)continue;
            let frame=this.frames.get(item.frame_id);
            const events=await decodeMirror(item);
            if(!frame||item.reset){
                if(!events.some(event=>event?.type===2))throw Error('missing_frame_snapshot');
                frame?.player.destroy();frame?.root.remove();
                const root=document.createElement('div');root.className='browser-next-frame';
                parent.root.append(root);
                const player=this.newPlayer(root,item.frame_id);
                const visualLayer=document.createElement('div');visualLayer.className='browser-next-frame-visuals';root.append(visualLayer);
                frame={root,player,visualLayer,visualNodes:new Map()};
                this.frames.set(item.frame_id,frame);
            }
            if(frame.root.parentNode!==parent.root)parent.root.append(frame.root);
            frame.latestVisuals=Array.isArray(item.visuals)?item.visuals:[];
            for(const event of events)frame.player.addEvent(fenceSnapshot(event));
            const position=this.framePosition(item,parent);
            frame.root.hidden=!position;
            if(position){
                frame.root.style.left=`${position.left}px`;frame.root.style.top=`${position.top}px`;
                frame.root.style.width=`${Math.max(0,position.width)}px`;
                frame.root.style.height=`${Math.max(0,position.height)}px`;
            }
            this.paintFrameVisuals(frame,frame.latestVisuals);
        }
        for(const [id,frame] of this.frames)if(!present.has(id)){
            frame.player.destroy();frame.root.remove();this.frames.delete(id);
        }
    }
    accept(status){
        const previous=this.status;
        if(staleBinding(previous?.binding,status?.binding)) return;
        if(previous && bindingKey(previous)!==bindingKey(status))this.discardQueued('input_fenced');
        if(previous && pageKey(previous)!==pageKey(status))this.clearMirror();
        this.status=status;
        if(this.attached&&Number.isSafeInteger(status.input_sequence))this.sequence=Math.max(this.sequence,status.input_sequence);
        if(!status.available)this.phase='unavailable';
        else if(!status.running)this.phase='stopped';
        else if(this.streaming)this.phase='live';
        else if(this.phase!=='connecting'&&this.phase!=='recovering')this.phase='waiting-page';
        this.emit();
    }
    async request(operation,epoch=this.epoch){
        if(this.closed)throw Error('viewer_closed');
        let timer;
        try{
            const reply=await Promise.race([this.transport(operation),new Promise((_,reject)=>{
                timer=setTimeout(()=>reject(Error('reply_timeout')),this.timeout);
            })]);
            if(this.closed||epoch!==this.epoch)throw Error('stale_viewer');
            if(!reply?.status||typeof reply.status.available!=='boolean'||typeof reply.status.running!=='boolean')throw Error('invalid_status');
            if(staleBinding(this.status?.binding,reply.status.binding))throw Error('stale_status');
            if(operation.action!=='input'||operation.sequence>=this.sequence)this.accept(reply.status);
            return reply;
        }finally{clearTimeout(timer);}
    }
    fail(kind){this.discardQueued('input_unconfirmed');this.issue=kind;this.phase=kind==='input-unknown'?'needs-review':'error';this.emit();}
    async connect({start=true}={}){
        if(this.busy||this.closed)return;
        const epoch=++this.epoch;this.busy=true;this.issue=null;this.phase='connecting';this.emit();
        try{
            await this.request({action:'status'},epoch);
            if(!this.status.available)return;
            if(!this.status.running){
                if(!start)return;
                const {incarnation,revision}=this.context();
                await this.request({action:'start',command_id:this.uuid(),incarnation,expected_revision:revision},epoch);
            }
            if(!this.attached){
                await this.request(this.operation('attach',{binding:{...this.status.binding,attachment_id:this.uuid()}}),epoch);
                this.sequence=Number.isSafeInteger(this.status.input_sequence)?this.status.input_sequence:0;
            }
            if(!this.attached)throw Error('attachment_missing');
            await this.pull(epoch);
            this.schedulePoll();
        }catch(error){this.lastError=String(error);if(epoch===this.epoch&&!this.closed&&!this.issue)this.fail('connect-unknown');}
        finally{if(epoch===this.epoch){this.busy=false;this.emit();}}
    }
    schedulePoll(delay=180){
        clearTimeout(this.pollTimer);
        if(this.closed||!this.attached||!this.status?.running)return;
        this.pollTimer=setTimeout(async()=>{
            if(this.polling){this.schedulePoll(180);return;}
            this.polling=true;
            try{await this.pull();this.issue=null;}
            catch(error){this.lastError=String(error);if(!this.closed){this.clearMirror();this.fail('page-unavailable');}}
            finally{this.polling=false;this.schedulePoll(this.streaming?260:600);}
        },delay);
    }
    async pull(epoch=this.epoch){
        if(!this.attached||!this.status?.running||this.closed)return;
        const binding={...this.status.binding}, key=pageKey(this.status), since=this.cursor;
        const reply=await this.request({action:'mirror',binding,since},epoch);
        if(key!==pageKey(this.status))return;
        const value=reply.value;
        if(!value||value.reset&&value.encoding!=='gzip'){
            this.clearMirror();return;
        }
        if(!Number.isSafeInteger(value.cursor)||value.cursor<since&&!value.reset)throw Error('invalid_cursor');
        const events=await decodeMirror(value);
        if(key!==pageKey(this.status))return;
        if(value.reset){
            this.clearMirror();
            if(!events.some(event=>event?.type===2))throw Error('missing_full_snapshot');
            this.replayer=this.newPlayer(this.mirror);
        }
        for(const event of events)this.replayer?.addEvent(fenceSnapshot(event));
        this.cursor=value.cursor;
        this.latestVisuals=Array.isArray(value.visuals)?value.visuals:[];
        await this.updateFrames(value.frames||[]);
        this.onVisuals(this.latestVisuals,this.replayer);
    }
    async refresh(){
        if(this.closed||this.refreshing||!this.status)return;
        this.refreshing=true;
        try{await this.request({action:'status'});if(this.attached&&this.status.running&&!this.pollTimer)this.schedulePoll();}
        catch{if(!this.closed)this.fail('status-unavailable');}
        finally{this.refreshing=false;}
    }
    async recover(){
        if(this.closed||this.busy)return;
        this.issue=null;this.clearMirror();
        if(!this.status||!this.attached||!this.status.running)return this.connect({start:false});
        this.phase='recovering';this.emit();
        try{await this.request({action:'status'});if(this.attached&&this.status.running)await this.pull();this.schedulePoll();}
        catch{this.fail('status-unavailable');}
    }
    async control(mode){
        if(!this.attached||this.busy||!['agent','human','private'].includes(mode))return;
        this.busy=true;this.phase='switching';this.emit();
        let committed=false;
        try{await this.request(this.operation('control',{mode}));committed=true;this.clearMirror();await this.pull();this.issue=null;}
        catch{this.fail(committed?'page-unavailable':'control-unknown');}
        finally{this.busy=false;this.emit();}
    }
    enqueue(input,resolve=null,reject=null){
        if(!this.canInput)return false;
        if(this.queue.length>=32){this.fail('input-unknown');return false;}
        this.queue.push({input,key:bindingKey(this.status),epoch:this.epoch,resolve,reject});
        void this.flush();return true;
    }
    input(input){return this.enqueue(input);}
    confirmedInput(input){return new Promise((resolve,reject)=>{
        if(!this.enqueue(input,resolve,reject))reject(Error('input_unavailable'));
    });}
    async urgentInput(input){
        const interrupt=input?.type==='dialog'&&this.controls&&!!this.status?.dialog
            || input?.type==='history'&&input.direction==='stop'&&this.controls;
        if((!this.canInput&&!interrupt)||this.urgentPromise)return false;
        this.discardQueued('input_superseded');
        if(this.sending){
            const action=this.operation('input',{sequence:++this.sequence,input}),epoch=this.epoch;
            const pending=this.request(action,epoch).then(()=>true).catch(()=>{
                if(epoch===this.epoch&&!this.closed)this.fail('input-unknown');return false;
            }).finally(()=>{if(this.urgentPromise===pending)this.urgentPromise=null;});
            this.urgentPromise=pending;return pending;
        }
        return new Promise(resolve=>{
            this.queue.unshift({input,key:bindingKey(this.status),epoch:this.epoch,
                resolve:()=>resolve(true),reject:()=>resolve(false)});void this.flush();
        });
    }
    async flush(){
        if(this.sending)return;
        this.sending=true;let next;
        try{while(this.queue.length){
            next=this.queue.shift();
            const interrupt=next.input?.type==='dialog'&&this.controls&&!!this.status?.dialog
                || next.input?.type==='history'&&next.input.direction==='stop'&&this.controls;
            if((!this.canInput&&!interrupt)||next.epoch!==this.epoch||next.key!==bindingKey(this.status)){
                next.reject?.(Error('input_fenced'));continue;
            }
            const reply=await this.request(this.operation('input',{sequence:++this.sequence,input:next.input}),next.epoch);
            next.resolve?.(reply.value);next=null;
            if(this.urgentPromise)await this.urgentPromise;
        }}catch{next?.reject?.(Error('input_unconfirmed'));if(!this.closed&&next?.epoch===this.epoch)this.fail('input-unknown');}
        finally{this.sending=false;this.emit();}
    }
    async closeBrowser(){
        if(!this.controls||this.busy)return;
        this.busy=true;this.emit();
        try{await this.request(this.operation('close'));this.clearMirror();this.issue=null;this.phase='stopped';}
        catch{this.fail('close-unknown');}
        finally{this.busy=false;this.emit();}
    }
    disconnect(){
        const detach=this.attached&&!this.closed?this.operation('detach'):null;
        this.epoch++;clearTimeout(this.pollTimer);this.pollTimer=null;this.clearMirror();
        this.discardQueued('viewer_disconnected');this.phase='disconnected';this.issue=null;this.emit();
        if(detach)void Promise.resolve().then(()=>this.transport(detach)).catch(()=>{});
    }
    dispose(){if(this.closed)return;this.disconnect();this.closed=true;this.emit();}
}

export function mountBrowserViewer(root,options={}){
    const doc=root.ownerDocument;
    root.classList.add('host-browser-viewer','browser-next');
    root.setAttribute('aria-label','Voyage browser');
    const node=(tag,parent,className='',content='')=>{
        const element=doc.createElement(tag);if(className)element.className=className;
        if(content)element.textContent=content;parent.append(element);return element;
    };
    const button=(label,parent,action,className='',content=label)=>{
        const element=node('button',parent,className,content);element.type='button';
        element.setAttribute('aria-label',label);element.title=label;element.addEventListener('click',action);return element;
    };
    const chrome=node('header',root,'browser-next-chrome');
    const identity=node('div',chrome,'browser-next-identity');
    node('span',identity,'browser-next-name','BROWSER');
    const status=node('span',identity,'browser-next-status','Opening…');status.setAttribute('role','status');status.setAttribute('aria-live','polite');
    const actions=node('div',chrome,'browser-next-actions');
    const primary=button('Take control privately',actions,()=>void connection.control(connection.controls?'agent':'private'),'browser-primary browser-next-primary');
    const scale=button('View at actual size',actions,()=>{actual=!actual;scaleChosen=true;paintScale();},'browser-next-scale','100%');
    const more=node('details',actions,'browser-next-more');
    const summary=node('summary',more,'','More');summary.setAttribute('aria-label','More browser options');
    const menu=node('div',more,'browser-next-menu');
    const downloads=node('div',menu,'browser-next-downloads');
    const disconnect=button('Disconnect viewer',menu,()=>{connection.disconnect();more.open=false;});
    const closeBrowser=button('Close browser',menu,()=>{void connection.closeBrowser();more.open=false;},'browser-next-danger');
    if(!options.externalClose)button('Close viewer',actions,()=>{dispose();options.onClose?.();},'browser-next-close','×');
    const tabs=node('div',root,'browser-next-tabs');tabs.setAttribute('role','group');tabs.setAttribute('aria-label','Browser tabs');
    const navigation=node('form',root,'browser-next-navigation');navigation.setAttribute('aria-label','Browser navigation');
    const back=button('Back',navigation,()=>void navigateAction({type:'history',direction:'back'}),'browser-next-icon','←');
    const forward=button('Forward',navigation,()=>void navigateAction({type:'history',direction:'forward'}),'browser-next-icon','→');
    const reload=button('Reload',navigation,()=>void navigateAction({type:'history',direction:connection.status?.page?.loading?'stop':'reload'}),'browser-next-icon','↻');
    const address=node('input',navigation,'browser-next-address');address.type='text';address.inputMode='url';
    address.placeholder='Search or enter a website address';address.setAttribute('aria-label','Website address');address.autocomplete='off';address.spellcheck=false;
    const go=button('Go to address',navigation,()=>{},'browser-next-go','Go');go.type='submit';
    navigation.addEventListener('submit',async event=>{
        event.preventDefault();let url=address.value.trim();if(!url)return;
        if(!/^https?:\/\//i.test(url))url=`https://${url}`;
        try{const parsed=new URL(url);if(!['https:','http:'].includes(parsed.protocol)||parsed.username||parsed.password||bytes(url)>8192)return;}
        catch{return;}
        if(await navigateAction({type:'navigate',url}))connection.replayer?.iframe.focus();
    });
    const stage=node('div',root,'browser-next-stage');
    const scroll=node('div',stage,'browser-next-scroll');
    const shell=node('div',scroll,'browser-next-mirror-shell');
    const mirror=node('div',shell,'browser-next-mirror');
    const visualLayer=node('div',shell,'browser-next-visuals');visualLayer.setAttribute('aria-hidden','true');
    const frameLayer=node('div',shell,'browser-next-frames');
    const uploadPicker=node('input',stage);uploadPicker.type='file';uploadPicker.hidden=true;
    let uploadNode=null;
    uploadPicker.addEventListener('change',async()=>{
        const file=uploadPicker.files?.[0],target=uploadNode;uploadPicker.value='';uploadNode=null;
        if(!file||!target||!connection.canInput||file.size>2*1024*1024)return;
        const raw=new Uint8Array(await file.arrayBuffer());
        let binary='';for(let offset=0;offset<raw.length;offset+=16384)binary+=String.fromCharCode(...raw.subarray(offset,offset+16384));
        const input={type:'upload',node_id:target.id,name:file.name,mime_type:file.type||'application/octet-stream',data_base64:btoa(binary)};
        try{await connection.confirmedInput(target.frameId?{type:'frame_element',frame_id:target.frameId,input}:input);}
        catch{}
    });
    const welcome=node('div',stage,'browser-next-welcome');
    node('span',welcome,'browser-next-welcome-mark','↗');node('h3',welcome,'','Your voyage’s browser');
    node('p',welcome,'','Enter an address above, or ask the agent to open a site. The browser stays with this voyage when you leave this view.');
    button('Browse privately',welcome,async()=>{await connection.control('private');address.focus();},'browser-next-welcome-action');
    const recovery=node('section',stage,'browser-next-recovery');recovery.hidden=true;recovery.setAttribute('role','status');
    const recoveryTitle=node('h3',recovery),recoveryBody=node('p',recovery);
    const recoveryAction=button('Check browser status',recovery,()=>void(connection.phase==='stopped'?connection.connect({start:true}):connection.recover()));
    const footer=node('footer',root,'browser-next-footer');
    const modeHint=node('span',footer,'browser-next-mode-hint');
    const compose=node('form',footer,'browser-next-compose');
    const composed=node('textarea',compose);composed.rows=1;composed.placeholder='Type or paste into the browser';
    composed.setAttribute('aria-label','Text for browser');composed.autocomplete='off';composed.spellcheck=false;
    const sendText=button('Send text to browser',compose,()=>{},'','Send');sendText.type='submit';
    compose.addEventListener('submit',async event=>{
        event.preventDefault();const value=composed.value;
        if(!value||bytes(value)>16384||!connection.canInput)return;
        sendText.disabled=true;
        try{await connection.confirmedInput({type:'text',text:value});if(composed.value===value)composed.value='';}
        catch{}finally{render();}
    });
    const dialogPanel=node('section',stage,'browser-next-dialog');dialogPanel.hidden=true;dialogPanel.setAttribute('aria-label','Website dialog');
    const dialogMessage=node('p',dialogPanel);
    const dialogInput=node('input',dialogPanel);dialogInput.setAttribute('aria-label','Website dialog response');
    button('Accept dialog',dialogPanel,()=>{void connection.urgentInput({type:'dialog',accept:true,text:dialogInput.value||null});dialogInput.value='';});
    button('Dismiss dialog',dialogPanel,()=>{void connection.urgentInput({type:'dialog',accept:false,text:null});dialogInput.value='';});
    let actual=false,scaleChosen=false,resizeTimer=null,resizeTarget='',disposed=false,tabFingerprint='',downloadFingerprint='',frameFence='';
    const visualNodes=new Map();
    const connection=new BrowserConnection({...options,mirror,frameLayer,onReplay,onVisuals,changed:()=>{render();options.changed?.(connection);}});
    function paintScale(){
        const viewport=connection.status?.viewport;
        if(!viewport?.width||!viewport?.height)return;
        const ratio=actual?1:Math.min(1,scroll.clientWidth/viewport.width,scroll.clientHeight/viewport.height);
        const factor=Number.isFinite(ratio)&&ratio>0?ratio:1;
        shell.style.width=`${Math.round(viewport.width*factor)}px`;
        shell.style.height=`${Math.round(viewport.height*factor)}px`;
        mirror.style.width=`${viewport.width}px`;mirror.style.height=`${viewport.height}px`;
        mirror.style.transform=`scale(${factor})`;
        visualLayer.style.width=`${viewport.width}px`;visualLayer.style.height=`${viewport.height}px`;
        visualLayer.style.transform=`scale(${factor})`;
        frameLayer.style.width=`${viewport.width}px`;frameLayer.style.height=`${viewport.height}px`;
        frameLayer.style.transform=`scale(${factor})`;
        root.dataset.scale=actual?'actual':'fit';scale.textContent=actual?'Fit':'100%';
        scale.setAttribute('aria-label',actual?'Fit page in view':'View at actual size');scale.title=scale.getAttribute('aria-label');
        scale.setAttribute('aria-pressed',String(actual));
    }
    async function navigateAction(input){
        if(!connection.attached||connection.busy)return false;
        if(!connection.controls)await connection.control('private');
        if(!connection.canInput)return false;
        if(input.type==='history'&&input.direction==='stop')return connection.urgentInput(input);
        try{await connection.confirmedInput(input);return true;}catch{return false;}
    }
    function scheduleViewport(){
        if(!connection.canInput||connection.status?.mode!=='private'||connection.status?.dialog||connection.sending||connection.queue.length)return;
        const width=Math.max(320,Math.min(3840,Math.round(scroll.clientWidth)));
        const height=Math.max(240,Math.min(2160,Math.round(scroll.clientHeight)));
        if(!width||!height||Math.abs((connection.status?.viewport?.width||0)-width)<20&&Math.abs((connection.status?.viewport?.height||0)-height)<20)return;
        const target=`${width}×${height}`;if(target===resizeTarget)return;
        resizeTarget=target;clearTimeout(resizeTimer);
        resizeTimer=setTimeout(async()=>{
            try{if(connection.canInput&&!connection.status?.dialog)await connection.confirmedInput({type:'resize',width,height});}
            catch{}finally{resizeTarget='';}
        },250);
    }
    function renderDownloads(){
        const fingerprint=JSON.stringify([connection.status?.downloads,connection.controls]);
        if(fingerprint===downloadFingerprint)return;
        downloadFingerprint=fingerprint;
        downloads.replaceChildren();
        for(const item of connection.status?.downloads||[]){
            const name=safe(item.name,255)||'download';
            button(`Download ${name}`,downloads,async()=>{
                if(!connection.controls)return;
                try{
                    const value=await connection.confirmedInput({type:'download',download_id:item.id});
                    if(!value?.data_base64||value.data_base64.length>2800000)return;
                    const data=Uint8Array.from(atob(value.data_base64),char=>char.charCodeAt(0));
                    const url=URL.createObjectURL(new Blob([data],{type:value.mime_type||'application/octet-stream'}));
                    const link=doc.createElement('a');link.href=url;link.download=safe(value.name,255)||'download';
                    doc.body.append(link);link.click();link.remove();setTimeout(()=>URL.revokeObjectURL(url),60000);
                }catch{}
                more.open=false;
            });
        }
        downloads.hidden=!downloads.childElementCount;
    }
    function renderTabs(){
        const current=connection.status,details=current?.tab_details||[];
        const key=JSON.stringify([current?.tabs,details,current?.binding?.tab_id,connection.busy,connection.attached]);
        if(key===tabFingerprint)return;tabFingerprint=key;tabs.replaceChildren();
        (current?.tabs||[]).forEach((id,index)=>{
            const detail=details.find(item=>item.id===id),title=safe(detail?.title||detail?.url)||`Tab ${index+1}`;
            const tab=node('div',tabs,'browser-next-tab');
            const select=button(title,tab,()=>void navigateAction({type:'tab',operation:'select',tab_id:id}));
            select.setAttribute('aria-pressed',String(id===current?.binding?.tab_id));select.disabled=!connection.attached||connection.busy;
            const close=button(`Close ${title}`,tab,()=>void navigateAction({type:'tab',operation:'close',tab_id:id}),'browser-next-tab-close','×');
            close.disabled=!connection.attached||connection.busy||(current?.tabs?.length||0)<2;
        });
        const add=button('New tab',tabs,()=>void navigateAction({type:'tab',operation:'new',tab_id:null}),'browser-next-new-tab','+');
        add.disabled=!connection.attached||connection.busy;
    }
    function render(){
        if(disposed)return;
        const current=connection.status,phase=connection.phase,privateControl=connection.controls&&current?.mode==='private';
        root.dataset.state=phase==='live'?(privateControl?'private':current?.mode==='agent'?'agent':'watching'):phase;
        mirror.style.pointerEvents=connection.canInput?'auto':'none';
        frameLayer.dataset.control=String(connection.canInput);
        status.textContent=phase==='live'?privateControl?'Private control · Agent paused':current?.agent_active?'Agent working':'Watching browser'
            :phase==='switching'?'Switching control…':phase==='connecting'?'Opening browser…':phase==='recovering'?'Restoring page…'
            :phase==='stopped'?'Browser stopped':phase==='unavailable'?'Browser unavailable':phase==='waiting-page'?'Loading page…'
            :phase==='needs-review'?'Action needs review':'Browser connection needs attention';
        primary.textContent=connection.controls?'Return to agent':'Take control privately';
        primary.setAttribute('aria-label',primary.textContent);primary.title=primary.textContent;
        primary.disabled=!connection.attached||connection.busy||connection.closed;
        closeBrowser.disabled=!connection.controls||connection.busy;
        disconnect.disabled=!connection.attached||connection.busy;
        const metadata=current?.mode==='private'&&!connection.controls?null:current,page=metadata?.page;
        if(doc.activeElement!==address)address.value=page?.url==='about:blank'?'':safe(page?.url,8192);
        address.disabled=!connection.attached||connection.busy;go.disabled=address.disabled;
        back.disabled=!connection.attached||connection.busy||page?.can_go_back!==true;
        forward.disabled=!connection.attached||connection.busy||page?.can_go_forward!==true;
        reload.disabled=!connection.attached||connection.busy;
        reload.textContent=page?.loading?'■':'↻';reload.setAttribute('aria-label',page?.loading?'Stop loading':'Reload');
        const blank=connection.attached&&(!page?.url||page.url==='about:blank');welcome.hidden=!blank||phase==='unavailable';
        recovery.hidden=!connection.issue&&phase!=='stopped'&&phase!=='unavailable'&&phase!=='disconnected';
        if(!recovery.hidden){
            const issue=connection.issue;
            recoveryTitle.textContent=issue==='input-unknown'?'Your last action was not confirmed':issue==='control-unknown'?'Control change was not confirmed'
                :issue==='page-unavailable'?'Live page stopped':phase==='stopped'?'This browser is stopped':phase==='unavailable'?'Browser unavailable on this host':'Could not confirm this voyage’s browser';
            recoveryBody.textContent=issue==='input-unknown'?'The page may have changed. Check its current state before acting again. Your action will not be repeated.'
                :issue==='page-unavailable'?'The voyage may still be running. Resynchronize the page without repeating actions.'
                :phase==='stopped'?'Start a browser for this voyage when it is available.'
                :phase==='unavailable'?'This Voyage host cannot start a browser with its current configuration.'
                :'Check the voyage state and try again.';
            recoveryAction.textContent=phase==='stopped'?'Start browser':'Check browser status';recoveryAction.disabled=connection.busy;
        }
        modeHint.textContent=privateControl?'Private: your input stays out of agent history. Return control when finished.'
            :connection.controls?'You control this browser.':'Take private control to type, paste, and navigate.';
        compose.hidden=!connection.controls;composed.disabled=!connection.canInput;sendText.disabled=!connection.canInput||!composed.value;
        const dialog=connection.controls?current?.dialog:null;
        dialogPanel.hidden=!dialog;dialogMessage.textContent=safe(dialog?.message,4096);dialogInput.hidden=dialog?.type!=='prompt';
        const fence=pageKey(current);if(fence!==frameFence){frameFence=fence;dialogInput.value='';}
        if(!scaleChosen)actual=privateControl&&scroll.clientWidth<680&&Math.abs((current?.viewport?.width||0)-scroll.clientWidth)<20;
        renderTabs();renderDownloads();paintScale();scheduleViewport();
    }
    function onReplay(player,session,frameId){
        const frame=player.iframe,body=frame.contentDocument;
        const send=input=>session.input(frameId?{type:'frame_element',frame_id:frameId,input}:input);
        const targetId=target=>{
            const element=target?.nodeType===1?target:target?.parentElement;
            return element?player.getMirror().getId(element):-1;
        };
        body.addEventListener('click',event=>{
            event.preventDefault();event.stopPropagation();
            if(!session.canInput)return;
            const id=targetId(event.target);
            if(event.target?.matches?.('input[type=file]')){
                if(id>0){uploadNode={id,frameId};uploadPicker.click();}
                return;
            }
            if(event.target?.matches?.('canvas,video,iframe')){
                const rect=event.target.getBoundingClientRect();
                if(id>0&&rect.width&&rect.height)send({type:'surface_click',node_id:id,
                    x:Math.max(0,Math.min(10000,Math.round((event.clientX-rect.left)/rect.width*10000))),
                    y:Math.max(0,Math.min(10000,Math.round((event.clientY-rect.top)/rect.height*10000))),button:'left'});
                return;
            }
            if(id>0)send({type:'click',node_id:id,button:'left'});
        },true);
        body.addEventListener('contextmenu',event=>{
            event.preventDefault();event.stopPropagation();
            if(!session.canInput)return;
            const id=targetId(event.target);
            if(id>0)send({type:'click',node_id:id,button:'right'});
        },true);
        body.addEventListener('input',event=>{
            if(!session.canInput||event.isComposing)return;
            const element=event.target,id=targetId(element);
            if(id<=0)return;
            if(element.matches?.('select')){send({type:'select',node_id:id,value:element.value});return;}
            if(element.matches?.('input:not([type=checkbox]):not([type=radio]):not([type=file]),textarea,[contenteditable]')){
                const text=element.isContentEditable?element.textContent:element.value;
                if(typeof text==='string'&&bytes(text)<=16384)send({type:'fill',node_id:id,text});
            }
        },true);
        body.addEventListener('wheel',event=>{
            if(!session.canInput)return;
            const id=targetId(event.target);if(id<=0)return;
            event.preventDefault();
            const factor=event.deltaMode===1?16:event.deltaMode===2?session.status?.viewport?.height||720:1;
            const bound=value=>Math.max(-16384,Math.min(16384,Math.round(value*factor)));
            send({type:'wheel',node_id:id,delta_x:bound(event.deltaX),delta_y:bound(event.deltaY)});
        },{capture:true,passive:false});
        const keys=new Set();
        const key=(event,pressed)=>{
            if(event.key==='Escape'){event.preventDefault();primary.focus();return;}
            if(!session.canInput||event.isComposing||['Process','Dead'].includes(event.key)||bytes(event.key)>128||/[\x00-\x1f\x7f]/.test(event.key))return;
            const editable=event.target?.matches?.('input,textarea,[contenteditable]');
            if(editable&&!['Enter','Tab','Escape','ArrowUp','ArrowDown'].includes(event.key))return;
            event.preventDefault();pressed?keys.add(event.key):keys.delete(event.key);
            session.input({type:'key',key:event.key,pressed});
        };
        body.addEventListener('keydown',event=>key(event,true),true);
        body.addEventListener('keyup',event=>key(event,false),true);
        frame.addEventListener('blur',()=>{for(const name of keys)session.input({type:'key',key:name,pressed:false});keys.clear();});
    }
    function onVisuals(items,player){
        if(!player||!Array.isArray(items)){
            visualLayer.replaceChildren();visualNodes.clear();return;
        }
        const present=new Set();
        for(const item of items){
            if(!Number.isSafeInteger(item.id)||typeof item.data_base64!=='string'||item.data_base64.length>270000)continue;
            const target=player.getMirror().getNode(item.id);
            if(!target?.getBoundingClientRect)continue;
            const rect=target.getBoundingClientRect();
            present.add(item.id);
            let picture=visualNodes.get(item.id);
            if(!picture){picture=node('img',visualLayer,'browser-next-visual');picture.alt='';visualNodes.set(item.id,picture);}
            if(picture.dataset.version!==String(item.version)){
                picture.src=`data:image/jpeg;base64,${item.data_base64}`;
                picture.dataset.version=String(item.version);
            }
            picture.style.left=`${Math.max(0,item.left??rect.left)}px`;picture.style.top=`${Math.max(0,item.top??rect.top)}px`;
            picture.style.width=`${Math.max(0,item.width)}px`;
            picture.style.height=`${Math.max(0,item.height)}px`;
        }
        for(const [id,picture] of visualNodes)if(!present.has(id)){picture.remove();visualNodes.delete(id);}
    }
    composed.addEventListener('input',()=>{sendText.disabled=!connection.canInput||!composed.value;});
    const observer=typeof ResizeObserver!=='undefined'?new ResizeObserver(()=>{paintScale();scheduleViewport();}):null;
    observer?.observe(scroll);
    const refresh=setInterval(()=>void connection.refresh(),2000);
    function dispose(){if(disposed)return;disposed=true;clearInterval(refresh);clearTimeout(resizeTimer);observer?.disconnect();connection.dispose();root.replaceChildren();}
    render();if(options.autoConnect!==false)void connection.connect();
    return {session:connection,disconnect:()=>connection.disconnect(),dispose};
}
