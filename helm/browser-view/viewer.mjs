import {mountCapture} from './capture.mjs';
const NIL = '00000000-0000-0000-0000-000000000000';
const fingerprint = binding => JSON.stringify(binding);
const mediaFence = status => JSON.stringify([status?.binding?.incarnation,status?.binding?.browser_id,status?.binding?.attachment_id,status?.binding?.capture_epoch,status?.binding?.controller_epoch,status?.mode,status?.controller]);
const bytes = value => new TextEncoder().encode(value).length;

export function videoPoint(video, clientX, clientY) {
    const rect = video.getBoundingClientRect(), width = video.videoWidth, height = video.videoHeight;
    if (!width || !height || !rect.width || !rect.height) return null;
    const scale = Math.min(rect.width / width, rect.height / height);
    const x = (clientX - rect.left - (rect.width - width * scale) / 2) / scale;
    const y = (clientY - rect.top - (rect.height - height * scale) / 2) / scale;
    return x < 0 || y < 0 || x >= width || y >= height ? null : {x:Math.floor(x), y:Math.floor(y)};
}

// All state is attachment-local and memory-only. Transport errors are deliberately
// reduced to fixed messages, never displayed/logged with their private payloads.
export class BrowserSession {
    constructor({transport, context, video, changed = () => {}, peer = config => new RTCPeerConnection(config), rtcConfiguration = {}, uuid = () => crypto.randomUUID(), timeout = 10000}) {
        Object.assign(this, {transport, context, video, changed, peer, rtcConfiguration, uuid, timeout});
        this.status = null; this.generation = 0; this.sequence = 0; this.queue = []; this.busy = false; this.closed = false; this.message = 'Browser disconnected';
    }
    notify(message) { if (message) this.message = message; this.changed(this); }
    clearVideo() {
        this.generation++; this.queue = []; clearTimeout(this.mediaTimer);
        const pc = this.pc; this.pc = null;
        if (pc) { pc.ontrack = null; pc.onconnectionstatechange = null; pc.close(); }
        if (this.video) { this.video.srcObject?.getTracks().forEach(track => track.stop()); this.video.srcObject = null; }
        this.streaming = false; this.streamRevision = (this.streamRevision || 0) + 1;
    }
    disconnect(message = 'Browser disconnected. Retry connection to resume.') {
        const detach = this.attached && this.status?.binding ? this.operation('detach') : null;
        this.clearVideo(); this.status = null; this.attached = false; this.notify(message);
        if (detach && !this.closed) Promise.resolve().then(() => this.transport(detach)).catch(() => {});
    }
    async call(operation, generation = this.generation) {
        if (this.closed) throw Error('closed');
        let timer;
        try {
            const reply = await Promise.race([this.transport(operation), new Promise((_, reject) => { timer = setTimeout(() => reject(Error('slow')), this.timeout); })]);
            if (this.closed || generation !== this.generation) throw Error('stale');
            if (!reply?.status || (reply.status.running && !['agent','human','private'].includes(reply.status.mode))) throw Error('invalid');
            this.accept(reply.status);
            return reply;
        } finally { clearTimeout(timer); }
    }
    accept(status) {
        const old = this.status;
        if (old && fingerprint(old.binding) !== fingerprint(status.binding)) this.queue = [];
        if (old && mediaFence(old) !== mediaFence(status)) this.clearVideo();
        this.status = status;
        this.attached = Boolean(status.binding && status.binding.attachment_id !== NIL);
        this.notify(!status.available ? 'Browser unavailable on this host' : !status.running ? 'Browser stopped' : `Browser · ${status.mode} control`);
    }
    operation(action, fields = {}) {
        return {action, command_id:this.uuid(), binding:{...this.status.binding}, ...fields};
    }
    get controls() {
        return this.attached && ['human','private'].includes(this.status?.mode) && this.status.controller === this.status.binding.attachment_id;
    }
    async exclusive(action) {
        if (this.busy || this.closed) return;
        this.busy = true; this.notify();
        try { return await action(); }
        catch { this.disconnect('Browser operation interrupted or unconfirmed. Nothing was replayed. Retry connection to refresh.'); }
        finally { this.busy = false; this.notify(); }
    }
    async refresh() {
        if (this.busy || this.closed || !this.status) return;
        await this.exclusive(async () => { await this.call({action:'status'}); if (this.attached && !this.pc && !this.pumping) await this.negotiate(); });
    }
    async connect() {
        return this.exclusive(async () => {
            this.clearVideo();
            await this.call({action:'status'});
            if (!this.status.available) return;
            if (!this.status.running) {
                const {incarnation, revision} = this.context();
                await this.call({action:'start',command_id:this.uuid(),incarnation,expected_revision:revision});
            }
            if (!this.attached) { await this.call(this.operation('attach', {binding:{...this.status.binding,attachment_id:this.uuid()}})); this.sequence = 0; }
            if (!this.attached) throw Error('attach');
            // A remounted viewer on the same socket must continue the acknowledged
            // attachment sequence, never guess zero or replay old input.
            this.sequence = Number.isSafeInteger(this.status.input_sequence) ? this.status.input_sequence : this.sequence;
            await this.negotiate();
        });
    }
    async negotiate() {
        const reply = await this.call(this.operation('signal', {signal:{type:'request_offer'}}));
        const generation = this.generation, binding = mediaFence(this.status);
        const current = () => !this.closed && generation === this.generation && binding === mediaFence(this.status);
        if (reply.value?.type !== 'offer' || typeof reply.value.sdp !== 'string' || bytes(reply.value.sdp) > 65536) throw Error('offer');
        const pc = this.pc = this.peer(reply.value.rtc_configuration ?? this.rtcConfiguration);
        this.mediaTimer = setTimeout(() => { if (current() && !this.streaming) this.disconnect('Live video did not arrive. Retry connection to resume.'); }, this.timeout);
        pc.ontrack = event => {
            if (!current() || pc !== this.pc) { event.track.stop(); return; }
            const stream = event.streams[0] || new MediaStream([event.track]);
            clearTimeout(this.mediaTimer); this.video.srcObject = stream; this.streaming = true;
            this.video.play()?.catch(() => { if (current()) this.disconnect('Video playback was blocked. Retry connection to resume.'); });
            this.notify();
        };
        pc.onconnectionstatechange = () => {
            if (current() && ['failed','closed','disconnected'].includes(pc.connectionState)) this.disconnect('Video connection lost. Retry connection to resume.');
        };
        await pc.setRemoteDescription({type:'offer',sdp:reply.value.sdp});
        if (!current()) return;
        const answer = await pc.createAnswer();
        if (!current()) return;
        await pc.setLocalDescription(answer);
        if (!current()) return;
        if (pc.iceGatheringState !== 'complete') await new Promise((resolve, reject) => {
            const timer = setTimeout(() => finish(Error('ice timeout')), this.timeout);
            const finish = error => { clearTimeout(timer); pc.removeEventListener('icegatheringstatechange', check); error ? reject(error) : resolve(); };
            const check = () => { if (!current()) finish(Error('stale')); else if (pc.iceGatheringState === 'complete') finish(); };
            pc.addEventListener('icegatheringstatechange', check); check();
        });
        if (!current()) return;
        const sdp = pc.localDescription?.sdp;
        if (!sdp || bytes(sdp) > 65536) throw Error('answer');
        await this.call(this.operation('signal',{signal:{type:'answer',sdp}}), generation);
    }
    async control(mode) {
        if (!this.attached || !['agent','human','private'].includes(mode)) return;
        return this.exclusive(async () => {
            this.clearVideo();
            await this.call(this.operation('control',{mode}));
            await this.negotiate();
        });
    }
    input(input) {
        if (!this.controls || this.busy || !this.streaming || this.closed) return false;
        if (this.queue.length >= 32) { this.disconnect('Browser input too slow. Input cleared; reconnect required.'); return false; }
        // Website modal replies must interrupt the pointer/navigation that opened
        // the dialog; putting them behind that operation deadlocks human control.
        if (input.type === 'dialog' || input.type === 'history' && input.direction === 'stop') {
            const generation = this.generation;
            void this.call(this.operation('input',{sequence:++this.sequence,input}),generation)
                .catch(() => { if (generation === this.generation) this.disconnect('Browser interruption unconfirmed; not replayed.'); });
            return true;
        }
        this.queue.push({input, binding:fingerprint(this.status.binding), generation:this.generation});
        this.pump(); return true;
    }
    async pump() {
        if (this.pumping) return;
        this.pumping = true;
        const generation = this.generation;
        try {
            while (this.queue.length) {
                const next = this.queue.shift();
                if (!this.controls || next.generation !== this.generation || next.binding !== fingerprint(this.status.binding)) continue;
                await this.call(this.operation('input',{sequence:++this.sequence,input:next.input}), next.generation);
            }
        } catch { if (generation === this.generation) this.disconnect('Input outcome unconfirmed. Input cleared; nothing replayed.'); }
        finally { this.pumping = false; this.notify(); }
    }
    dispose() {
        if (this.closed) return;
        const detach = this.attached ? this.operation('detach') : null;
        this.closed = true; this.disconnect();
        // Best effort only; server socket loss is the authoritative cleanup path.
        if (detach) Promise.resolve().then(() => this.transport(detach)).catch(() => {});
    }
}

// Host metadata is always inert text; never load page HTML, icons or URLs locally.
const displayText = (value, limit = 512) => typeof value === 'string'
    ? value.replace(/[\x00-\x1f\x7f-\x9f\u202a-\u202e\u2066-\u2069]/g, '').slice(0, limit) : '';

export function mountBrowserViewer(root, options = {}) {
    const doc = root.ownerDocument;
    root.classList.add('host-browser-viewer');
    root.setAttribute('aria-label', 'Host browser');
    const element = (tag, text, parent = root, className = '') => {
        const node = doc.createElement(tag); if (text) node.textContent = text;
        if (className) node.className = className; parent.append(node); return node;
    };
    const sensitive = [];
    const button = (label, action, parent, control = false, glyph = null) => {
        const node = element('button', glyph || label, parent); node.type = 'button';
        node.setAttribute('aria-label', label); node.title = label; node.onclick = action;
        if (control) sensitive.push(node); return node;
    };
    const header = element('div', null, root, 'browser-header');
    const status = element('span', 'Disconnected', header, 'browser-status');
    status.setAttribute('role', 'status'); status.setAttribute('aria-live', 'polite');
    let capture;
    const captureButton = button('Capture and annotate', () => capture?.open(), header);
    const primary = button('Take control privately', () => session.control(session.controls && session.status?.mode === 'private' ? 'agent' : 'private'), header);
    primary.className = 'browser-primary';
    const more = element('details', null, header, 'browser-more');
    const summary = element('summary', 'More', more); summary.setAttribute('aria-label', 'More browser options');
    const menu = element('div', null, more, 'browser-menu');
    const textLabel = element('label', 'Compose text (IME)', menu);
    const text = element('textarea', null, textLabel); text.rows = 2; text.autocomplete = 'off'; text.spellcheck = false;
    text.setAttribute('aria-label', 'Text for remote browser'); sensitive.push(text);
    button('Send text', () => { const value = text.value; text.value = ''; if (value && bytes(value) <= 16384) session.input({type:'text',text:value}); }, menu, true);
    const sizeLabel = element('label', 'Resolution', menu);
    const size = element('select', null, sizeLabel); size.setAttribute('aria-label', 'Remote viewport size'); sensitive.push(size);
    for (const value of ['1280×720','1920×1080','1024×768','640×480']) { const option = element('option', value, size); option.value = value; }
    button('Resize', () => { const [width,height] = size.value.split('×').map(Number); session.input({type:'resize',width,height}); }, menu, true);
    const closeBrowser = button('Close browser', () => session.exclusive(async () => {
        if (!session.controls) return; session.clearVideo(); await session.call(session.operation('close'));
    }), menu, true);
    closeBrowser.className = 'browser-destructive';
    button('Disconnect viewer', () => { session.disconnect(); more.open = false; }, menu);
    if (!options.externalClose) button('Close viewer', () => { dispose(); options.onClose?.(); }, header, false, '×');
    more.addEventListener('keydown', event => { if (event.key === 'Escape') { event.preventDefault(); more.open = false; summary.focus(); } });
    const tabs = element('div', null, root, 'browser-tabs'); tabs.setAttribute('role', 'group'); tabs.setAttribute('aria-label', 'Browser tabs');
    const form = element('form', null, root, 'browser-navigation'); form.setAttribute('aria-label', 'Browser navigation');
    const back = button('Back', () => session.input({type:'history',direction:'back'}), form, true, '←');
    const forward = button('Forward', () => session.input({type:'history',direction:'forward'}), form, true, '→');
    const reload = button('Reload', () => session.input({type:'history',direction:session.status?.page?.loading ? 'stop' : 'reload'}), form, true, '↻');
    const address = element('input', null, form, 'browser-address'); address.type = 'url'; address.placeholder = 'Enter an https:// address';
    address.setAttribute('aria-label', 'Address'); address.autocomplete = 'off'; address.spellcheck = false;
    const go = button('Go', () => {}, form, true); go.type = 'submit';
    form.onsubmit = event => {
        event.preventDefault(); const url = address.value;
        if (/^https?:\/\//.test(url) && bytes(url) <= 8192 && !/[\x00-\x1f\x7f]/.test(url) && session.input({type:'navigate',url})) { address.value = ''; video.focus(); }
    };
    const privacy = element('p', 'Taking control is private: agent observation is paused until you return control.', root, 'browser-privacy');
    const viewport = element('div', null, root, 'browser-viewport');
    const video = element('video', null, viewport); video.autoplay = true; video.muted = true; video.playsInline = true; video.tabIndex = 0;
    video.setAttribute('aria-label', 'Remote browser. Escape releases keyboard focus; use More to compose text.');
    const cursor = element('span', '↖', viewport, 'browser-agent-cursor'); cursor.hidden = true; cursor.setAttribute('aria-hidden','true');
    const empty = element('div', null, viewport, 'browser-empty');
    const explanation = element('p', 'Opening browser…', empty);
    const retry = button('Retry connection', () => session.connect(), empty);
    const startPage = element('section',null,viewport,'browser-start-page');
    element('h3','Where would you like to go?',startPage);
    element('p','Ask the agent to open a site, or take private control and enter an address above.',startPage);
    const useAddress = button('Enter an address',async()=>{ if(!session.controls)await session.control('private'); if(session.controls){address.focus();address.select();} },startPage);
    const recent = element('div',null,startPage,'browser-recent');
    const recentPages = new Map(); let recentKey='';

    const dialogPanel = element('section', null, root, 'browser-dialog'); dialogPanel.hidden = true;
    dialogPanel.setAttribute('role', 'region'); dialogPanel.setAttribute('aria-label', 'Website dialog');
    const dialogMessage = element('p', null, dialogPanel); dialogMessage.setAttribute('role', 'status');
    const dialog = element('input', null, dialogPanel); dialog.setAttribute('aria-label', 'Website dialog response'); dialog.autocomplete = 'off'; sensitive.push(dialog);
    button('Accept dialog', () => { const value = dialog.value; dialog.value = ''; if (bytes(value) <= 16384) session.input({type:'dialog',accept:true,text:value || null}); }, dialogPanel, true);
    button('Dismiss dialog', () => { dialog.value = ''; session.input({type:'dialog',accept:false,text:null}); }, dialogPanel, true);
    let tabFingerprint = '', previousFence = '', dialogFingerprint = '';
    const keys = new Set();
    const session = new BrowserSession({...options,video,changed:current => {
        const attached = current.attached && current.status?.running;
        const privateControl = current.controls && current.status?.mode === 'private';
        status.textContent = !attached ? 'Disconnected' : privateControl ? 'You control privately' : current.status?.mode === 'agent' && current.status?.agent_active !== false ? 'Agent working' : 'Watching';
        root.dataset.state = !attached ? 'disconnected' : privateControl ? 'private' : current.status?.mode === 'agent' ? 'agent' : 'watching';
        primary.textContent = privateControl ? 'Return to agent' : 'Take control privately';
        primary.setAttribute('aria-label', primary.textContent); primary.title = primary.textContent;
        primary.disabled = !attached || current.busy || current.closed;
        captureButton.disabled = !current.streaming || current.busy || current.closed;
        if (!current.streaming) capture?.reset();
        privacy.textContent = privateControl ? 'Private control · Agent observation is paused. Return to agent explicitly when finished.' : 'Taking control is private: agent observation is paused until you return control.';
        const metadata = current.status?.mode === 'private' && !current.controls ? null : current.status;
        const actionLabels={inspect:'Reading page',navigate:'Opening page',click:'Clicking',fill:'Filling a field',scroll:'Scrolling',tabs:'Changing tabs',screenshot:'Capturing page',upload:'Uploading',download:'Downloading'};
        if(current.status?.agent_active&&actionLabels[current.status?.agent_action]) status.textContent='Agent · '+actionLabels[current.status.agent_action];
        const point=current.status?.mode==='agent'?current.status.agent_cursor:null;
        cursor.hidden=!point||Date.now()-point.at>2500||!current.streaming;
        if(!cursor.hidden&&video.videoWidth){const box=video.getBoundingClientRect(),wrap=viewport.getBoundingClientRect();const scale=Math.min(box.width/video.videoWidth,box.height/video.videoHeight);cursor.style.left=`${box.left-wrap.left+(box.width-video.videoWidth*scale)/2+point.x/point.width*video.videoWidth*scale}px`;cursor.style.top=`${box.top-wrap.top+(box.height-video.videoHeight*scale)/2+point.y/point.height*video.videoHeight*scale}px`;}
        const blank=attached&&(!metadata?.page?.url||metadata.page.url==='about:blank');
        startPage.hidden=!blank||current.status?.mode==='private'&&!current.controls;
        useAddress.disabled=current.busy||!attached;
        // Only nonprivate public-origin browsing is kept, in this viewer's memory.
        if(metadata?.page?.url&&current.status?.mode!=='private'&&metadata.page.url!=='about:blank'){
            try{const u=new URL(metadata.page.url);if(['https:','http:'].includes(u.protocol)&&!u.username&&!u.password){recentPages.set(u.origin,{url:u.origin,title:displayText(metadata.page.title)||u.hostname});if(recentPages.size>5)recentPages.delete(recentPages.keys().next().value);}}catch{}
        }
        const nextRecent=JSON.stringify([...recentPages.values()]);
        if(nextRecent!==recentKey){recentKey=nextRecent;recent.replaceChildren();if(recentPages.size){element('h4','Recent sites in this view',recent);for(const item of [...recentPages.values()].reverse())button(item.title,async()=>{if(!session.controls)await session.control('private');session.input({type:'navigate',url:item.url});},recent);}}
        const enabled = current.controls && !current.busy && current.streaming;
        for (const node of sensitive) node.disabled = !enabled;
        address.readOnly = !enabled; address.disabled = !attached;
        back.disabled = !enabled || metadata?.page?.can_go_back !== true;
        forward.disabled = !enabled || metadata?.page?.can_go_forward !== true;
        reload.textContent = metadata?.page?.loading ? '■' : '↻';
        reload.setAttribute('aria-label', metadata?.page?.loading ? 'Stop loading' : 'Reload'); reload.title = reload.getAttribute('aria-label');
        const fence = fingerprint(current.status?.binding) + mediaFence(current.status) + current.streamRevision;
        if (fence !== previousFence) { capture?.reset(); previousFence = fence; text.value = dialog.value = address.value = ''; keys.clear(); more.open = false; }
        if (doc.activeElement !== address) address.value = displayText(metadata?.page?.url, 8192);
        empty.hidden = Boolean(current.streaming);
        explanation.textContent = current.busy ? 'Opening browser…' : !attached ? current.message : current.status?.mode === 'private' && !current.controls ? 'Private control is active. Agent observation is paused.' : 'Waiting for live video…';
        retry.hidden = Boolean(attached) || current.busy; retry.disabled = current.busy || current.closed;
        const remoteDialog = current.controls ? current.status?.dialog : null;
        const nextDialog = JSON.stringify([fence,remoteDialog]);
        if (nextDialog !== dialogFingerprint) { dialogFingerprint = nextDialog; dialog.value = ''; }
        dialogPanel.hidden = !remoteDialog;
        dialogMessage.textContent = displayText(remoteDialog?.message, 4096);
        dialog.hidden = remoteDialog?.type !== 'prompt';
        const details = metadata?.tab_details || [];
        const key = JSON.stringify([current.status?.tabs,details,enabled,current.status?.binding?.tab_id]);
        if (key !== tabFingerprint) {
            const focused = doc.activeElement?.dataset?.tabAction;
            tabFingerprint = key; tabs.replaceChildren();
            (current.status?.tabs || []).forEach((id,index) => {
                const detail = details.find(item => item.id === id);
                const title = displayText(detail?.title) || displayText(detail?.url) || `Tab ${index + 1}`;
                const item = element('div', null, tabs, 'browser-tab');
                const node = button(title, () => session.input({type:'tab',operation:'select',tab_id:id}), item);
                node.dataset.tabAction = `select:${id}`; node.setAttribute('aria-pressed', String(id === current.status?.binding?.tab_id)); node.disabled = !enabled;
                const close = button(`Close ${title}`, () => session.input({type:'tab',operation:'close',tab_id:id}), item, false, '×');
                close.dataset.tabAction = `close:${id}`; close.disabled = !enabled;
            });
            const add = button('New tab', () => session.input({type:'tab',operation:'new',tab_id:null}), tabs, false, '+'); add.disabled = !enabled; add.dataset.tabAction = 'new';
            if (focused) [...tabs.querySelectorAll('button')].find(node => node.dataset.tabAction === focused)?.focus();
        }
        options.changed?.(current);
    }});
    capture = mountCapture({root,video,canCapture:()=>session.streaming&&!session.closed,onCapture:options.onCapture});
    const pointer = (event, pressed, moving = false) => {
        if (!session.controls) return;
        const point = videoPoint(video,event.clientX,event.clientY); if (!point) return;
        event.preventDefault();
        if (!moving && pressed) { video.focus(); video.setPointerCapture?.(event.pointerId); }
        session.input({type:'pointer',...point,button:moving ? null : ['left','middle','right'][event.button] || null,pressed});
    };
    video.onpointerdown = event => pointer(event,true); video.onpointerup = event => pointer(event,false);
    let moveTimer;
    video.onpointermove = event => { if (moveTimer) return; moveTimer = setTimeout(() => { moveTimer = null; },40); pointer(event,false,true); };
    video.oncontextmenu = event => event.preventDefault();
    video.addEventListener('wheel',event => { if (!session.controls) return; event.preventDefault(); const factor = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? video.videoHeight : 1; const clamp = value => Math.max(-16384,Math.min(16384,Math.round(value * factor))); session.input({type:'scroll',delta_x:clamp(event.deltaX),delta_y:clamp(event.deltaY)}); },{passive:false});
    const key = (event, pressed) => {
        if (event.key === 'Escape') { event.preventDefault(); primary.focus(); return; }
        if (!session.controls || event.isComposing || event.key === 'Process' || event.key === 'Dead') return;
        if (bytes(event.key) > 128 || /[\x00-\x1f\x7f]/.test(event.key)) return;
        event.preventDefault();
        if (pressed) keys.add(event.key); else keys.delete(event.key);
        session.input({type:'key',key:event.key,pressed});
    };
    video.onkeydown = event => key(event,true); video.onkeyup = event => key(event,false);
    video.onblur = () => { for (const key of keys) session.input({type:'key',key,pressed:false}); keys.clear(); };
    const hidden = () => { if (doc.hidden) session.disconnect('Viewer hidden. Retry connection to resume.'); };
    doc.addEventListener('visibilitychange',hidden);
    const timer = setInterval(() => session.refresh(),2000);
    let disposed = false;
    const dispose = () => { if (disposed) return; disposed = true; clearInterval(timer); clearTimeout(moveTimer); doc.removeEventListener('visibilitychange',hidden); capture.dispose(); session.dispose(); recentPages.clear(); root.replaceChildren(); };
    session.notify();
    if (options.autoConnect !== false) void session.connect();
    return {session,disconnect:() => session.disconnect(),dispose};
}
