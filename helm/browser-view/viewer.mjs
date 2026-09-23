import {mountCapture} from './capture.mjs';
const NIL = '00000000-0000-0000-0000-000000000000';
const bytes = value => new TextEncoder().encode(value).length;
const peerKey = status => JSON.stringify([
    status?.binding?.incarnation, status?.binding?.browser_id,
    status?.binding?.attachment_id, status?.binding?.tab_id,
]);
const bindingKey = status => JSON.stringify(status?.binding);

export function screenPoint(video, x, y, viewport) {
    const rect = video.getBoundingClientRect();
    if (!rect.width || !rect.height || !video.videoWidth || !video.videoHeight) return null;
    const scale = Math.min(rect.width / video.videoWidth, rect.height / video.videoHeight);
    const left = rect.left + (rect.width - video.videoWidth * scale) / 2;
    const top = rect.top + (rect.height - video.videoHeight * scale) / 2;
    const px = (x - left) / scale, py = (y - top) / scale;
    if (px < 0 || py < 0 || px >= video.videoWidth || py >= video.videoHeight) return null;
    const width = Number.isSafeInteger(viewport?.width) ? viewport.width : video.videoWidth;
    const height = Number.isSafeInteger(viewport?.height) ? viewport.height : video.videoHeight;
    return {x: Math.min(width - 1, Math.floor(px / video.videoWidth * width)),
        y: Math.min(height - 1, Math.floor(py / video.videoHeight * height))};
}

// One mounted viewer owns one attachment. Commands are never replayed after an
// unknown result; a read-only status request is the only automatic recovery.
export class BrowserConnection {
    constructor({transport, context, video, changed = () => {}, peer = config => new RTCPeerConnection(config),
        rtcConfiguration = {}, uuid = () => crypto.randomUUID(), timeout = 25000}) {
        Object.assign(this, {transport, context, video, changed, peer, rtcConfiguration, uuid, timeout});
        this.status = null; this.phase = 'idle'; this.issue = null; this.sequence = 0;
        this.queue = []; this.sending = false; this.urgentPromise = null; this.busy = false; this.closed = false;
        this.epoch = 0; this.mediaEpoch = 0; this.lastFrame = 0; this.streaming = false;
        this.wasHidden = !!video?.ownerDocument?.hidden;
    }
    emit() { this.changed(this); }
    operation(action, fields = {}) { return {action, command_id:this.uuid(), binding:{...this.status.binding}, ...fields}; }
    get attached() { return !!this.status?.binding && this.status.binding.attachment_id !== NIL; }
    get controls() { return this.attached && ['human','private'].includes(this.status?.mode)
        && this.status.controller === this.status.binding.attachment_id; }
    get canInput() { return this.controls && this.streaming && !this.busy && !this.issue && !this.closed; }
    discardQueued(reason) {
        this.queue.splice(0).forEach(item => item.reject?.(Error(reason)));
    }
    accept(status) {
        const previous = this.status;
        if (previous && bindingKey(previous) !== bindingKey(status)) this.discardQueued('input_fenced');
        if (previous && peerKey(previous) !== peerKey(status)) this.clearMedia();
        this.status = status;
        if (this.attached && Number.isSafeInteger(status.input_sequence)) this.sequence = Math.max(this.sequence,status.input_sequence);
        if (!status.available) this.phase = 'unavailable';
        else if (!status.running) this.phase = 'stopped';
        else if (this.streaming) this.phase = 'live';
        else if (this.phase !== 'connecting' && this.phase !== 'recovering') this.phase = 'waiting-video';
        this.emit();
    }
    async request(operation, epoch = this.epoch) {
        if (this.closed) throw Error('viewer_closed');
        let timer;
        try {
            const reply = await Promise.race([this.transport(operation), new Promise((_, reject) => {
                timer = setTimeout(() => reject(Error('reply_timeout')), this.timeout);
            })]);
            if (this.closed || epoch !== this.epoch) throw Error('stale_viewer');
            if (!reply?.status || typeof reply.status.available !== 'boolean' || typeof reply.status.running !== 'boolean') throw Error('invalid_status');
            // An interrupt can finish before the page action that opened its
            // native dialog. Do not roll the UI back to that older reply.
            if (operation.action !== 'input' || operation.sequence >= this.sequence) this.accept(reply.status);
            return reply;
        } finally { clearTimeout(timer); }
    }
    fail(kind) {
        this.discardQueued('input_unconfirmed');
        this.issue = kind;
        this.phase = kind === 'input-unknown' ? 'needs-review' : 'error';
        this.emit();
    }
    clearMedia() {
        this.mediaEpoch++;
        clearTimeout(this.mediaDeadline); clearTimeout(this.disconnectDeadline);
        this.rejectFrame?.(Error('media_closed')); this.rejectFrame = null;
        this.stopFrameWatch?.(); this.stopFrameWatch = null;
        const pc = this.pc; this.pc = null;
        if (pc) { pc.ontrack = null; pc.onconnectionstatechange = null; pc.close(); }
        if (this.video) { this.video.srcObject?.getTracks().forEach(track => track.stop()); this.video.srcObject = null; }
        this.streaming = false; this.lastFrame = 0;
    }
    async connect({start = true} = {}) {
        if (this.busy || this.closed) return;
        const epoch = ++this.epoch; this.busy = true; this.issue = null; this.phase = 'connecting'; this.emit();
        try {
            await this.request({action:'status'},epoch);
            if (!this.status.available) return;
            if (!this.status.running) {
                if (!start) return;
                const {incarnation,revision} = this.context();
                await this.request({action:'start',command_id:this.uuid(),incarnation,expected_revision:revision},epoch);
            }
            if (!this.attached) {
                await this.request(this.operation('attach',{binding:{...this.status.binding,attachment_id:this.uuid()}}),epoch);
                this.sequence = Number.isSafeInteger(this.status.input_sequence) ? this.status.input_sequence : 0;
            }
            if (!this.attached) throw Error('attachment_missing');
            await this.startMedia(epoch);
        } catch {
            if (epoch === this.epoch && !this.closed && !this.issue) this.fail('connect-unknown');
        } finally { if (epoch === this.epoch) { this.busy = false; this.emit(); } }
    }
    async startMedia(epoch = this.epoch) {
        if (!this.attached || !this.status?.running || this.closed) return;
        this.clearMedia(); this.phase = 'waiting-video'; this.emit();
        const reply = await this.request(this.operation('signal',{signal:{type:'request_offer'}}),epoch);
        if (reply.value?.type !== 'offer' || typeof reply.value.sdp !== 'string' || bytes(reply.value.sdp) > 65536) throw Error('invalid_offer');
        const key = peerKey(this.status), mediaEpoch = this.mediaEpoch;
        const current = () => !this.closed && epoch === this.epoch && mediaEpoch === this.mediaEpoch && key === peerKey(this.status);
        const pc = this.pc = this.peer(reply.value.rtc_configuration ?? this.rtcConfiguration);
        let ready, rejectReady;
        const firstFrame = new Promise((resolve,reject) => { ready = resolve; rejectReady = reject; });
        void firstFrame.catch(() => {});
        this.rejectFrame = rejectReady;
        const frame = () => {
            if (!current() || !this.video.videoWidth || !this.video.videoHeight || this.video.readyState < 2) return;
            this.lastFrame = Date.now(); this.streaming = true; this.phase = 'live'; this.issue = null;
            clearTimeout(this.mediaDeadline); this.rejectFrame = null; ready(); this.emit();
        };
        this.mediaDeadline = setTimeout(() => rejectReady(Error('first_frame_timeout')), this.timeout);
        pc.ontrack = event => {
            if (!current()) { event.track.stop(); return; }
            this.video.srcObject = event.streams[0] || new MediaStream([event.track]);
            this.video.addEventListener('loadeddata',frame); this.video.addEventListener('resize',frame);
            const watch = () => {
                if (!current()) return;
                frame();
                if (this.video.requestVideoFrameCallback) this.frameCallback = this.video.requestVideoFrameCallback(watch);
            };
            if (this.video.requestVideoFrameCallback) this.frameCallback = this.video.requestVideoFrameCallback(watch);
            this.stopFrameWatch = () => {
                this.video.removeEventListener('loadeddata',frame); this.video.removeEventListener('resize',frame);
                if (this.video.cancelVideoFrameCallback && this.frameCallback) this.video.cancelVideoFrameCallback(this.frameCallback);
            };
            this.video.play()?.catch(() => rejectReady(Error('playback_blocked')));
            frame();
        };
        pc.onconnectionstatechange = () => {
            if (!current()) return;
            if (pc.connectionState === 'connected') { clearTimeout(this.disconnectDeadline); if (this.streaming) this.phase = 'live'; this.emit(); }
            else if (pc.connectionState === 'disconnected') {
                this.phase = 'recovering'; this.emit();
                clearTimeout(this.disconnectDeadline);
                this.disconnectDeadline = setTimeout(() => { if (current() && pc.connectionState === 'disconnected') this.mediaFailed(); },5000);
            } else if (['failed','closed'].includes(pc.connectionState)) this.mediaFailed();
        };
        try {
            await pc.setRemoteDescription({type:'offer',sdp:reply.value.sdp});
            if (!current()) return;
            await pc.setLocalDescription(await pc.createAnswer());
            if (!current()) return;
            if (pc.iceGatheringState !== 'complete') await new Promise((resolve,reject) => {
                const timer = setTimeout(() => done(Error('ice_timeout')),this.timeout);
                const done = error => {clearTimeout(timer);pc.removeEventListener('icegatheringstatechange',check);error ? reject(error) : resolve();};
                const check = () => {if (!current()) done(Error('stale_viewer')); else if (pc.iceGatheringState === 'complete') done();};
                pc.addEventListener('icegatheringstatechange',check);check();
            });
            if (!current()) return;
            const sdp = pc.localDescription?.sdp;
            if (!sdp || bytes(sdp)>65536) throw Error('invalid_answer');
            await this.request(this.operation('signal',{signal:{type:'answer',sdp}}),epoch);
            await firstFrame;
        } catch (error) {
            if (current()) {this.clearMedia();this.fail('video-unavailable');}
            throw error;
        }
    }
    mediaFailed() {this.clearMedia();this.fail('video-unavailable');}
    async refresh() {
        if (this.closed || this.busy || this.refreshing || this.sending || this.queue.length) return;
        if (!this.status) return;
        this.refreshing = true;
        const epoch = this.epoch;
        try {
            await this.request({action:'status'},epoch);
            if (this.attached && this.status.running && !this.pc && !this.issue) await this.startMedia(epoch);
            const hidden=!!this.video.ownerDocument?.hidden;
            if(hidden)this.wasHidden=true;
            else if(this.wasHidden){this.lastFrame=Date.now();this.wasHidden=false;}
            if (this.streaming && this.lastFrame && !hidden && Date.now()-this.lastFrame > 8000) this.mediaFailed();
        } catch { if (epoch === this.epoch && !this.closed) this.fail('status-unavailable'); }
        finally { this.refreshing = false; }
    }
    async recover() {
        if (this.closed || this.busy) return;
        this.issue = null;
        if (!this.status || !this.attached || !this.status.running) return this.connect({start:false});
        this.phase = 'recovering'; this.emit();
        try {await this.request({action:'status'});if(this.attached && this.status.running && !this.pc)await this.startMedia();}
        catch {this.fail('status-unavailable');}
    }
    async control(mode) {
        if (!this.attached || this.busy || !['agent','human','private'].includes(mode)) return;
        this.busy = true; this.phase = 'switching'; this.emit();
        try {
            await this.request(this.operation('control',{mode}));
            // The new media owner keeps this authorized receiver. A tab/browser
            // change can still replace the peer and is reconciled here.
            if (!this.pc) await this.startMedia();
            this.issue = null;
        } catch { this.fail('control-unknown'); }
        finally {this.busy = false;this.emit();}
    }
    enqueue(input, resolve = null, reject = null) {
        if (!this.canInput) return false;
        if (this.queue.length >= 32) {this.fail('input-unknown');return false;}
        this.queue.push({input,key:bindingKey(this.status),epoch:this.epoch,resolve,reject});
        void this.flush(); return true;
    }
    input(input) {return this.enqueue(input);}
    confirmedInput(input) {
        return new Promise((resolve,reject) => {
            if (!this.enqueue(input,resolve,reject)) reject(Error('input_unavailable'));
        });
    }
    async urgentInput(input) {
        if (!this.canInput || this.urgentPromise) return false;
        this.discardQueued('input_superseded');
        if (this.sending) {
            // Native page dialogs can block the in-flight pointer/navigation
            // reply. The worker has a separate interrupt lane for this input.
            const action=this.operation('input',{sequence:++this.sequence,input});
            const epoch=this.epoch;
            const pending=this.request(action,epoch).then(()=>true).catch(()=>{
                if(epoch===this.epoch&&!this.closed)this.fail('input-unknown');return false;
            })
                .finally(()=>{if(this.urgentPromise===pending)this.urgentPromise=null;});
            this.urgentPromise=pending;
            return pending;
        }
        return new Promise(resolve => {
            this.queue.unshift({input,key:bindingKey(this.status),epoch:this.epoch,
                resolve:() => resolve(true),reject:() => resolve(false)});
            void this.flush();
        });
    }
    async flush() {
        if (this.sending) return;
        this.sending = true;
        let next;
        try {
            while (this.queue.length) {
                next = this.queue.shift();
                if (!this.canInput || next.epoch !== this.epoch || next.key !== bindingKey(this.status)) {next.reject?.(Error('input_fenced'));continue;}
                await this.request(this.operation('input',{sequence:++this.sequence,input:next.input}),next.epoch);
                next.resolve?.(true);next = null;
                if(this.urgentPromise)await this.urgentPromise;
            }
        } catch {next?.reject?.(Error('input_unconfirmed'));if(!this.closed&&next?.epoch===this.epoch)this.fail('input-unknown');}
        finally {this.sending = false;this.emit();}
    }
    async closeBrowser() {
        if (!this.controls || this.busy) return;
        this.busy = true;this.emit();
        try {await this.request(this.operation('close'));this.clearMedia();this.issue=null;this.phase='stopped';}
        catch {this.fail('close-unknown');}
        finally {this.busy=false;this.emit();}
    }
    disconnect() {
        const detach = this.attached && !this.closed ? this.operation('detach') : null;
        this.epoch++;this.clearMedia();this.discardQueued('viewer_disconnected');this.phase = 'disconnected';this.issue = null;this.emit();
        if (detach) void Promise.resolve().then(() => this.transport(detach)).catch(() => {});
    }
    dispose() {if(this.closed)return;this.disconnect();this.closed=true;this.emit();}
}


const safe = (value, limit = 512) => typeof value === 'string'
    ? value.replace(/[\x00-\x1f\x7f-\x9f\u202a-\u202e\u2066-\u2069]/g,'').slice(0,limit) : '';

export function mountBrowserViewer(root, options = {}) {
    const doc = root.ownerDocument;
    root.classList.add('host-browser-viewer','browser-next');
    root.setAttribute('aria-label','Voyage browser');
    const node = (tag, parent, className = '', content = '') => {
        const element = doc.createElement(tag);
        if (className) element.className = className;
        if (content) element.textContent = content;
        parent.append(element);return element;
    };
    const button = (label, parent, action, className = '', content = label) => {
        const element = node('button',parent,className,content);
        element.type = 'button';element.setAttribute('aria-label',label);element.title = label;
        element.addEventListener('click',action);return element;
    };
    const chrome = node('header',root,'browser-next-chrome');
    const identity = node('div',chrome,'browser-next-identity');
    node('span',identity,'browser-next-name','BROWSER');
    const status = node('span',identity,'browser-next-status','Opening…');
    status.setAttribute('role','status');status.setAttribute('aria-live','polite');
    const actions = node('div',chrome,'browser-next-actions');
    const primary = button('Take control privately',actions,() => void connection.control(connection.controls ? 'agent':'private'),'browser-primary browser-next-primary');
    const scale = button('View at actual size',actions,() => {actual = !actual;scaleChosen=true;paintScale();},'browser-next-scale','100%');
    const more = node('details',actions,'browser-next-more');
    const summary = node('summary',more,'','More');summary.setAttribute('aria-label','More browser options');
    const menu = node('div',more,'browser-next-menu');
    const captureButton = button('Capture and annotate',menu,() => {capture?.open();more.open=false;});
    const disconnect = button('Disconnect viewer',menu,() => {connection.disconnect();more.open=false;});
    const closeBrowser = button('Close browser',menu,() => {void connection.closeBrowser();more.open=false;},'browser-next-danger');
    if (!options.externalClose) button('Close viewer',actions,() => {dispose();options.onClose?.();},'browser-next-close','×');

    const tabs = node('div',root,'browser-next-tabs');tabs.setAttribute('role','group');tabs.setAttribute('aria-label','Browser tabs');
    const navigation = node('form',root,'browser-next-navigation');navigation.setAttribute('aria-label','Browser navigation');
    const back = button('Back',navigation,() => void navigateAction({type:'history',direction:'back'}),'browser-next-icon','←');
    const forward = button('Forward',navigation,() => void navigateAction({type:'history',direction:'forward'}),'browser-next-icon','→');
    const reload = button('Reload',navigation,() => void navigateAction({type:'history',direction:connection.status?.page?.loading?'stop':'reload'}),'browser-next-icon','↻');
    const address = node('input',navigation,'browser-next-address');address.type='text';address.inputMode='url';
    address.placeholder='Search or enter a website address';address.setAttribute('aria-label','Website address');
    address.autocomplete='off';address.spellcheck=false;
    const go = button('Go to address',navigation,() => {},'browser-next-go','Go');go.type='submit';
    navigation.addEventListener('submit',async event => {
        event.preventDefault();
        let url = address.value.trim();if(!url)return;
        if (!/^https?:\/\//i.test(url)) url = `https://${url}`;
        try {const parsed=new URL(url);if(!['https:','http:'].includes(parsed.protocol)||parsed.username||parsed.password||bytes(url)>8192)return;}
        catch {return;}
        if (await navigateAction({type:'navigate',url})) video.focus();
    });

    const stage = node('div',root,'browser-next-stage');
    const scroll = node('div',stage,'browser-next-scroll');
    const video = node('video',scroll,'browser-next-video');
    video.autoplay=true;video.muted=true;video.playsInline=true;video.tabIndex=0;
    video.setAttribute('aria-label','Remote browser. Escape returns to browser controls.');
    const cursor = node('span',stage,'browser-next-cursor','↖');cursor.hidden=true;cursor.setAttribute('aria-hidden','true');
    const welcome = node('div',stage,'browser-next-welcome');
    node('span',welcome,'browser-next-welcome-mark','↗');
    node('h3',welcome,'','Your voyage’s browser');
    node('p',welcome,'','Enter an address above, or ask the agent to open a site. The browser stays with this voyage when you leave this view.');
    button('Browse privately',welcome,async () => {await connection.control('private');address.focus();},'browser-next-welcome-action');
    const recovery = node('section',stage,'browser-next-recovery');recovery.hidden=true;
    recovery.setAttribute('role','status');
    const recoveryTitle = node('h3',recovery);
    const recoveryBody = node('p',recovery);
    const recoveryAction = button('Check browser status',recovery,() => void (connection.phase==='stopped'?connection.connect({start:true}):connection.recover()));

    const footer = node('footer',root,'browser-next-footer');
    const modeHint = node('span',footer,'browser-next-mode-hint');
    const compose = node('form',footer,'browser-next-compose');
    const composed = node('textarea',compose);composed.rows=1;composed.placeholder='Type or paste into the browser';
    composed.setAttribute('aria-label','Text for remote browser');composed.autocomplete='off';composed.spellcheck=false;
    const sendText = button('Send text to browser',compose,() => {},'','Send');sendText.type='submit';
    compose.addEventListener('submit',async event => {
        event.preventDefault();const value=composed.value;
        if (!value || bytes(value)>16384 || !connection.canInput) return;
        sendText.disabled=true;
        try {await connection.confirmedInput({type:'text',text:value});if(composed.value===value)composed.value='';video.focus();}
        catch { /* Keep the text so the user can inspect the page before deciding. */ }
        finally {render();}
    });
    const dialogPanel = node('section',stage,'browser-next-dialog');dialogPanel.hidden=true;
    dialogPanel.setAttribute('aria-label','Website dialog');
    const dialogMessage = node('p',dialogPanel);
    const dialogInput = node('input',dialogPanel);dialogInput.setAttribute('aria-label','Website dialog response');
    button('Accept dialog',dialogPanel,() => {void connection.urgentInput({type:'dialog',accept:true,text:dialogInput.value||null});dialogInput.value='';});
    button('Dismiss dialog',dialogPanel,() => {void connection.urgentInput({type:'dialog',accept:false,text:null});dialogInput.value='';});

    let actual=false, scaleChosen=false, capture=null, resizeTimer=null, resizeTarget='', disposed=false, tabFingerprint='', frameFence='';
    const connection = new BrowserConnection({...options,video,changed:() => {render();options.changed?.(connection);}});
    capture = mountCapture({root,video,canCapture:()=>connection.streaming&&!connection.closed,onCapture:options.onCapture});
    function paintScale() {
        const viewport=connection.status?.viewport;
        const useActual=actual&&viewport?.width&&viewport?.height;
        root.dataset.scale=useActual?'actual':'fit';
        video.style.width=useActual?`${viewport.width}px`:'';
        video.style.height=useActual?`${viewport.height}px`:'';
        scale.textContent=actual?'Fit':'100%';
        scale.setAttribute('aria-label',actual?'Fit page in view':'View at actual size');
        scale.title=scale.getAttribute('aria-label');
        scale.setAttribute('aria-pressed',String(actual));
    }
    async function navigateAction(input) {
        if (!connection.attached || connection.busy) return false;
        if (!connection.controls) await connection.control('private');
        if (!connection.canInput) return false;
        if (input.type==='history'&&input.direction==='stop') return connection.urgentInput(input);
        try {await connection.confirmedInput(input);return true;}catch{return false;}
    }
    function scheduleViewport() {
        if (!connection.canInput || connection.status?.mode!=='private'
            || connection.status?.dialog || connection.sending || connection.queue.length) return;
        const width=Math.max(320,Math.min(3840,Math.round(scroll.clientWidth)));
        const height=Math.max(240,Math.min(2160,Math.round(scroll.clientHeight)));
        if (!width || !height || Math.abs((connection.status?.viewport?.width||0)-width)<20 && Math.abs((connection.status?.viewport?.height||0)-height)<20) return;
        const target=`${width}×${height}`;if(target===resizeTarget)return;
        resizeTarget=target;clearTimeout(resizeTimer);
        resizeTimer=setTimeout(async()=>{
            try {if(connection.canInput&&!connection.status?.dialog)await connection.confirmedInput({type:'resize',width,height});}
            catch {} finally {resizeTarget='';}
        },250);
    }
    function renderTabs() {
        const current=connection.status;
        const details=current?.tab_details||[];
        const key=JSON.stringify([current?.tabs,details,current?.binding?.tab_id,connection.busy,connection.attached]);
        if(key===tabFingerprint)return;tabFingerprint=key;tabs.replaceChildren();
        (current?.tabs||[]).forEach((id,index)=>{
            const detail=details.find(item=>item.id===id);
            const title=safe(detail?.title||detail?.url)||`Tab ${index+1}`;
            const tab=node('div',tabs,'browser-next-tab');
            const select=button(title,tab,() => void navigateAction({type:'tab',operation:'select',tab_id:id}));
            select.setAttribute('aria-pressed',String(id===current?.binding?.tab_id));
            select.disabled=!connection.attached||connection.busy;
            const close=button(`Close ${title}`,tab,() => void navigateAction({type:'tab',operation:'close',tab_id:id}),'browser-next-tab-close','×');
            close.disabled=!connection.attached||connection.busy||(current?.tabs?.length||0)<2;
        });
        const add=button('New tab',tabs,() => void navigateAction({type:'tab',operation:'new',tab_id:null}),'browser-next-new-tab','+');
        add.disabled=!connection.attached||connection.busy;
    }
    function render() {
        if(disposed)return;
        const current=connection.status, phase=connection.phase, privateControl=connection.controls&&current?.mode==='private';
        root.dataset.state=phase==='live'?(privateControl?'private':current?.mode==='agent'?'agent':'watching'):phase;
        status.textContent=phase==='live' ? privateControl?'Private control · Agent paused':current?.agent_active?'Agent working':'Watching browser'
            :phase==='switching'?'Switching control…':phase==='connecting'?'Opening browser…':phase==='recovering'?'Restoring video…'
            :phase==='stopped'?'Browser stopped':phase==='unavailable'?'Browser unavailable':phase==='waiting-video'?'Waiting for video…'
            :phase==='needs-review'?'Action needs review':'Browser connection needs attention';
        primary.textContent=privateControl||connection.controls?'Return to agent':'Take control privately';
        primary.setAttribute('aria-label',primary.textContent);primary.title=primary.textContent;
        primary.disabled=!connection.attached||connection.busy||connection.closed;
        captureButton.disabled=!connection.streaming||connection.busy;
        closeBrowser.disabled=!connection.controls||connection.busy;
        disconnect.disabled=!connection.attached||connection.busy;
        if(!connection.streaming)capture?.reset();
        const metadata=current?.mode==='private'&&!connection.controls?null:current;
        const page=metadata?.page;
        if(doc.activeElement!==address)address.value=page?.url==='about:blank'?'':safe(page?.url,8192);
        address.disabled=!connection.attached||connection.busy;
        go.disabled=address.disabled;
        back.disabled=!connection.attached||connection.busy||page?.can_go_back!==true;
        forward.disabled=!connection.attached||connection.busy||page?.can_go_forward!==true;
        reload.disabled=!connection.attached||connection.busy;
        reload.textContent=page?.loading?'■':'↻';reload.setAttribute('aria-label',page?.loading?'Stop loading':'Reload');
        const blank=connection.attached&&(!page?.url||page.url==='about:blank');
        welcome.hidden=!blank||phase==='unavailable';
        recovery.hidden=!connection.issue&&phase!=='stopped'&&phase!=='unavailable'&&phase!=='disconnected';
        if(!recovery.hidden){
            const issue=connection.issue;
            recoveryTitle.textContent=issue==='input-unknown'?'Your last action was not confirmed':issue==='control-unknown'?'Control change was not confirmed'
                :issue==='video-unavailable'?'Live video stopped':phase==='stopped'?'This browser is stopped':phase==='unavailable'?'Browser unavailable on this host':'Could not confirm this voyage’s browser';
            recoveryBody.textContent=issue==='input-unknown'?'The page may have changed. Check its current state before acting again. Your action will not be repeated.'
                :issue==='video-unavailable'?'The voyage may still be running. Recheck the media connection without repeating page actions.'
                :phase==='stopped'?'Start a browser for this voyage when it is available.'
                :phase==='unavailable'?'This Voyage host cannot start a browser with its current configuration.'
                :'The Vessel connection alone does not confirm this voyage or its browser. Check the voyage state and try again.';
            recoveryAction.textContent=phase==='stopped'?'Start browser':'Check browser status';
            recoveryAction.disabled=connection.busy;
        }
        modeHint.textContent=privateControl?'Private: your input stays out of agent history. Return control when finished.'
            :connection.controls?'You control this browser.':'Take private control to type, paste, and navigate.';
        compose.hidden=!connection.controls;
        composed.disabled=!connection.canInput;sendText.disabled=!connection.canInput||!composed.value;
        const dialog=connection.controls?current?.dialog:null;
        dialogPanel.hidden=!dialog;dialogMessage.textContent=safe(dialog?.message,4096);dialogInput.hidden=dialog?.type!=='prompt';
        const fence=JSON.stringify([current?.binding?.browser_id,current?.binding?.tab_id,current?.binding?.controller_epoch,current?.mode]);
        if(fence!==frameFence){frameFence=fence;capture?.reset();dialogInput.value='';}
        const point=current?.mode==='agent'?current?.agent_cursor:null;
        cursor.hidden=!point||Date.now()-point.at>2500||!connection.streaming;
        if(!cursor.hidden&&current?.viewport){cursor.style.left=`${point.x/current.viewport.width*100}%`;cursor.style.top=`${point.y/current.viewport.height*100}%`;}
        if(!scaleChosen)actual=privateControl&&scroll.clientWidth<680
            &&Math.abs((current?.viewport?.width||0)-scroll.clientWidth)<20;
        renderTabs();paintScale();scheduleViewport();
    }
    const pointer = (event, pressed, moving=false) => {
        if(!connection.canInput)return;
        const point=screenPoint(video,event.clientX,event.clientY,connection.status?.viewport);if(!point)return;
        event.preventDefault();if(pressed){video.focus();video.setPointerCapture?.(event.pointerId);}
        connection.input({type:'pointer',...point,button:moving?null:['left','middle','right'][event.button]||null,pressed});
    };
    video.onpointerdown=event=>pointer(event,true);video.onpointerup=event=>pointer(event,false);
    let moveAt=0;video.onpointermove=event=>{if(Date.now()-moveAt<40)return;moveAt=Date.now();pointer(event,false,true);};
    video.oncontextmenu=event=>event.preventDefault();
    video.addEventListener('wheel',event=>{
        if(!connection.canInput)return;event.preventDefault();
        const factor=event.deltaMode===1?16:event.deltaMode===2?connection.status?.viewport?.height||720:1;
        const bounded=value=>Math.max(-16384,Math.min(16384,Math.round(value*factor)));
        connection.input({type:'scroll',delta_x:bounded(event.deltaX),delta_y:bounded(event.deltaY)});
    },{passive:false});
    const keys=new Set();
    const key=(event,pressed)=>{
        if(event.key==='Escape'){event.preventDefault();primary.focus();return;}
        if(!connection.canInput||event.isComposing||['Process','Dead'].includes(event.key)||bytes(event.key)>128||/[\x00-\x1f\x7f]/.test(event.key))return;
        event.preventDefault();pressed?keys.add(event.key):keys.delete(event.key);
        connection.input({type:'key',key:event.key,pressed});
    };
    video.onkeydown=event=>key(event,true);video.onkeyup=event=>key(event,false);
    video.onblur=()=>{for(const name of keys)connection.input({type:'key',key:name,pressed:false});keys.clear();};
    composed.addEventListener('input',()=>{sendText.disabled=!connection.canInput||!composed.value;});
    const observer=typeof ResizeObserver!=='undefined'?new ResizeObserver(()=>{
        if(!scaleChosen){actual=connection.controls&&connection.status?.mode==='private'&&scroll.clientWidth<680
            &&Math.abs((connection.status?.viewport?.width||0)-scroll.clientWidth)<20;paintScale();}
        scheduleViewport();
    }):null;observer?.observe(scroll);
    const refresh=setInterval(()=>void connection.refresh(),2000);
    function dispose(){if(disposed)return;disposed=true;clearInterval(refresh);clearTimeout(resizeTimer);observer?.disconnect();capture?.dispose();connection.dispose();root.replaceChildren();}
    render();if(options.autoConnect!==false)void connection.connect();
    return {session:connection,disconnect:()=>connection.disconnect(),dispose};
}

export {BrowserConnection as BrowserSession, screenPoint as videoPoint};
