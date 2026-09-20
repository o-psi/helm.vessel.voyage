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
        this.generation++; this.queue = [];
        const pc = this.pc; this.pc = null;
        if (pc) { pc.ontrack = null; pc.onconnectionstatechange = null; pc.close(); }
        if (this.video) { this.video.srcObject?.getTracks().forEach(track => track.stop()); this.video.srcObject = null; }
        this.streaming = false;
    }
    disconnect(message = 'Browser disconnected. Connect explicitly to resume.') {
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
        catch { this.disconnect('Browser operation interrupted or unconfirmed. Nothing was replayed. Connect to refresh.'); }
        finally { this.busy = false; this.notify(); }
    }
    async refresh() {
        if (this.busy || this.pumping || this.closed || !this.status) return;
        await this.exclusive(async () => { await this.call({action:'status'}); if (this.attached && !this.pc) await this.negotiate(); });
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
        pc.ontrack = event => {
            if (!current() || pc !== this.pc) { event.track.stop(); return; }
            const stream = event.streams[0] || new MediaStream([event.track]);
            this.video.srcObject = stream; this.streaming = true;
            this.video.play()?.catch(() => { if (current()) this.notify('Use the video play control to start playback'); });
            this.notify();
        };
        pc.onconnectionstatechange = () => {
            if (current() && ['failed','closed','disconnected'].includes(pc.connectionState)) this.disconnect('Video connection lost. Connect explicitly to resume.');
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
        if (input.type === 'dialog') {
            const generation = this.generation;
            void this.call(this.operation('input',{sequence:++this.sequence,input}),generation)
                .catch(() => { if (generation === this.generation) this.disconnect('Dialog response unconfirmed; not replayed.'); });
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

export function mountBrowserViewer(root, options) {
    const doc = root.ownerDocument;
    root.classList.add('host-browser-viewer');
    const element = (tag, text, parent = root) => { const node = doc.createElement(tag); if (text) node.textContent = text; parent.append(node); return node; };
    const toolbar = element('div'); toolbar.className = 'browser-toolbar';
    const status = element('p'); status.setAttribute('role','status'); status.setAttribute('aria-live','polite');
    const sensitive = [];
    const button = (label, action, parent = toolbar, control = false) => { const node = element('button',label,parent); node.type = 'button'; node.onclick = action; if (control) sensitive.push(node); return node; };
    const connect = button('Start / Connect', () => session.connect());
    const human = button('Take control', () => session.control('human'));
    const privateMode = button('Private control', () => session.control('private'));
    const agent = button('Return to agent', () => session.control('agent'));
    button('Disconnect viewer', () => { session.dispose(); options.onClose?.(); });
    button('Close browser', () => session.exclusive(async () => { if (!session.attached) return; session.clearVideo(); await session.call(session.operation('close')); }));
    const form = element('form'); form.className = 'browser-toolbar';
    const address = element('input',null,form); address.type = 'url'; address.placeholder = 'https://…'; address.setAttribute('aria-label','Navigate browser'); address.autocomplete = 'off'; sensitive.push(address);
    const go = button('Go', () => {}, form, true); go.type = 'submit';
    form.onsubmit = event => { event.preventDefault(); const url = address.value; address.value = ''; if (/^https?:\/\//.test(url) && bytes(url) <= 8192 && !/[\x00-\x1f\x7f]/.test(url)) session.input({type:'navigate',url}); };
    const tabs = element('div'); tabs.className = 'browser-toolbar'; tabs.setAttribute('aria-label','Browser tabs');
    const video = element('video'); video.autoplay = true; video.muted = true; video.playsInline = true; video.tabIndex = 0; video.setAttribute('aria-label','Remote browser video; focus for keyboard control');
    const editor = element('div'); editor.className = 'browser-toolbar';
    const text = element('textarea',null,editor); text.rows = 1; text.placeholder = 'Type or compose text (IME)'; text.setAttribute('aria-label','Text for remote browser'); text.autocomplete = 'off'; text.spellcheck = false; sensitive.push(text);
    button('Send text', () => { const value = text.value; text.value = ''; if (value && bytes(value) <= 16384) session.input({type:'text',text:value}); },editor,true);
    const dialog = element('input',null,editor); dialog.placeholder = 'Dialog response (optional)'; dialog.setAttribute('aria-label','Remote dialog response'); dialog.autocomplete = 'off'; sensitive.push(dialog);
    button('Accept dialog', () => { const value = dialog.value; dialog.value = ''; if (bytes(value) <= 16384) session.input({type:'dialog',accept:true,text:value || null}); },editor,true);
    button('Dismiss dialog', () => { dialog.value = ''; session.input({type:'dialog',accept:false,text:null}); },editor,true);
    const size = element('select',null,editor); size.setAttribute('aria-label','Remote viewport size'); sensitive.push(size);
    for (const value of ['1280×720','1920×1080','1024×768','640×480']) { const option = element('option',value,size); option.value = value; }
    button('Resize', () => { const [width,height] = size.value.split('×').map(Number); session.input({type:'resize',width,height}); },editor,true);
    let tabFingerprint = '', previousFence = '';
    const session = new BrowserSession({...options,video,changed:current => {
        status.textContent = current.message + (current.status?.mode === 'private' ? ' · Private: agent observation withheld; return explicitly.' : '');
        connect.disabled = current.busy || current.closed;
        human.disabled = privateMode.disabled = !current.attached || current.busy;
        agent.disabled = !current.controls || current.busy;
        for (const node of sensitive) node.disabled = !current.controls || current.busy || !current.streaming;
        const fence = fingerprint(current.status?.binding);
        if (fence !== previousFence) { previousFence = fence; text.value = dialog.value = address.value = ''; }
        const key = JSON.stringify([current.status?.tabs,current.controls,current.busy,current.streaming,current.status?.binding?.tab_id]);
        if (key !== tabFingerprint) {
            tabFingerprint = key; tabs.replaceChildren();
            button('New tab', () => session.input({type:'tab',operation:'new',tab_id:null}),tabs).disabled = !current.controls || current.busy || !current.streaming;
            (current.status?.tabs || []).forEach((id,index) => {
                const node = button(`Tab ${index + 1}`, () => session.input({type:'tab',operation:'select',tab_id:id}),tabs);
                node.setAttribute('aria-pressed',String(id === current.status?.binding?.tab_id)); node.disabled = !current.controls || current.busy || !current.streaming;
                button(`Close tab ${index + 1}`, () => session.input({type:'tab',operation:'close',tab_id:id}),tabs).disabled = node.disabled;
            });
        }
        options.changed?.(current);
    }});
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
    const keys = new Set();
    const key = (event, pressed) => {
        if (!session.controls || event.isComposing || event.key === 'Process' || event.key === 'Dead') return;
        if (bytes(event.key) > 128 || /[\x00-\x1f\x7f]/.test(event.key)) return;
        event.preventDefault();
        if (pressed) keys.add(event.key); else keys.delete(event.key);
        session.input({type:'key',key:event.key,pressed});
    };
    video.onkeydown = event => key(event,true); video.onkeyup = event => key(event,false);
    video.onblur = () => { for (const key of keys) session.input({type:'key',key,pressed:false}); keys.clear(); };
    const hidden = () => { if (doc.hidden) session.disconnect('Viewer hidden. Connect explicitly to resume.'); };
    doc.addEventListener('visibilitychange',hidden);
    const timer = setInterval(() => session.refresh(),2000);
    session.notify();
    return {session,disconnect:() => session.disconnect(),dispose:() => { clearInterval(timer); clearTimeout(moveTimer); doc.removeEventListener('visibilitychange',hidden); session.dispose(); root.replaceChildren(); }};
}
