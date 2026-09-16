import {marked} from 'marked';
import DOMPurify from 'dompurify';
import {request, voyageResult, mutation, resolved, IntentJournal, VesselSocket} from './vessel-client.js';

const clean = value => String(value ?? '').replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/g, '');
export function markdown(text) {
    // No remote images, raw HTML controls, embedded content or executable links.
    return DOMPurify.sanitize(marked.parse(clean(text)), {ALLOWED_TAGS: ['p','br','strong','em','del','code','pre','blockquote','ul','ol','li','h1','h2','h3','h4','hr','a','table','thead','tbody','tr','th','td'], ALLOWED_ATTR: ['href','title'], ALLOW_DATA_ATTR: false});
}
function element(tag, text, className) { const node = document.createElement(tag); if (tag === 'pre') node.className = 'whitespace-pre-wrap break-words text-xs leading-6'; if (text != null) node.textContent = clean(text); if (className) node.className = className; return node; }
function fluxTemplate(id, label) {
    const template = document.getElementById(id);
    if (!template) throw new Error(`Missing Flux template: ${id}`);
    const node = template.content.firstElementChild.cloneNode(true);
    if (label != null) node.querySelector('[data-label]').textContent = clean(label);
    return node;
}
function button(label, action) { const node = fluxTemplate('flux-action', label); node.type = 'button'; node.addEventListener('click', action); return node; }

export function mount(root) {
    const $ = id => root.querySelector(`#${id}`);
    let client, journal, selected = null, snapshot = null, incarnation = null, generation = 0, stale = true, busy = false, refreshing = false;
    let voyages = [], messages = [], decisions = [], earliest = 0, revision = null, renewal, reconnectTimer, stopped = false, retry = 1000, lastCatalogue = 0;
    let messageFingerprint = '', decisionFingerprint = '', lastFresh = 0, outputFingerprint = '', outputOffset = null;
    const state = text => { $('connection-state').textContent = text; };
    const notice = text => { $('notice').textContent = clean(text); };
    const running = () => ['running','starting','cancelling'].includes(snapshot?.run?.state);
    const pending = () => journal?.entries().filter(e => e.session_id === selected) || [];
    const actionable = () => client && snapshot && !stale && !busy && Date.now() - lastFresh < 5000 && !pending().length;
    function controls() {
        try {
            const enabled = actionable();
            $('vessel').disabled = busy; $('reconnect').disabled = busy; $('prompt').disabled = !enabled; $('send').disabled = !enabled; $('cancel').disabled = !enabled || !running();
            $('send').textContent = running() ? 'Steer run' : 'Send';
            $('pending').replaceChildren();
            for (const entry of pending()) $('pending').append(fluxTemplate('flux-text', `${entry.op} · ${entry.command_id} · outcome not confirmed. Receipt checks only; never automatically resent.`));
            for (const node of $('decisions').querySelectorAll('button,input')) node.disabled = !enabled || Number(node.closest('.decision').dataset.expires) <= Date.now();
        } catch { stale = true; $('send').disabled = true; $('cancel').disabled = true; notice('Browser command journal unavailable. Sending is disabled to prevent uncertain duplicate work.'); }
    }
    async function ticket(alias) {
        const response = await fetch(root.dataset.ticketUrl, {method: 'POST', credentials: 'same-origin', headers: {'Content-Type':'application/json','Accept':'application/json','X-CSRF-TOKEN':document.querySelector('meta[name="csrf-token"]').content}, body: JSON.stringify({vessel:alias})});
        if (!response.ok) { if ([401,403,419].includes(response.status)) stopped = true; throw new Error('Unable to authorize connection. Sign in again if your session expired.'); }
        return (await response.json()).ticket;
    }
    async function connect() {
        clearTimeout(reconnectTimer); clearInterval(renewal); generation++; const mine = generation;
        client?.close(); client = null; stale = true; refreshing = false; state('Connecting…'); controls();
        const alias = $('vessel').value;
        if (!alias) { state('No Vessels connected'); notice('Add your first Vessel using Manage Vessels.'); return; }
        try {
            const auth = await ticket(alias); if (mine !== generation) return;
            const url = new URL(root.dataset.socketPath, location.href); url.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
            const socket = new WebSocket(url);
            const ready = await new Promise((resolve, reject) => {
                const timer = setTimeout(() => { socket.close(); reject(new Error('Gateway connection timed out.')); }, 10000);
                socket.addEventListener('open', () => socket.send(JSON.stringify({type:'authenticate',ticket:auth})));
                socket.addEventListener('message', function receive(event) {
                    try { const frame = JSON.parse(event.data); if (frame.type === 'ready') { clearTimeout(timer); socket.removeEventListener('message', receive); resolve(frame); } } catch { socket.close(); }
                });
                socket.addEventListener('close', () => { clearTimeout(timer); reject(new Error('Gateway connection refused or unavailable.')); }, {once:true});
                socket.addEventListener('error', () => { socket.close(); });
            });
            if (mine !== generation || stopped) { socket.close(); return; }
            try { if (typeof ready.vessel_id !== 'string') throw new Error('Gateway identity missing.'); journal = new IntentJournal(localStorage, `${root.dataset.tenantId}:${alias}:${ready.vessel_id}`); journal.entries(); } catch (error) { socket.close(); stopped = true; throw error; }
            client = new VesselSocket(socket, () => {
                if (mine !== generation) return;
                client = null; stale = true; clearInterval(renewal); state('Disconnected · stale view'); controls(); schedule();
            });
            retry = 1000; state('Connected'); lastCatalogue = 0;
            renewal = setInterval(async () => { try { const token = await ticket(alias); if (mine === generation && socket.readyState === 1) socket.send(JSON.stringify({type:'authenticate',ticket:token})); } catch (e) { notice(e.message); socket.close(); } }, 30000);
            await refresh();
        } catch (error) { if (mine !== generation) return; notice(error.message); state('Disconnected'); schedule(); }
    }
    function schedule() { if (!stopped) { clearTimeout(reconnectTimer); reconnectTimer = setTimeout(connect, retry); retry = Math.min(retry * 2, 15000); } }
    function renderVoyages() {
        const query = $('voyage-search').value.toLowerCase(); $('voyages').replaceChildren();
        for (const item of voyages.filter(v => `${v.name || ''} ${v.session_id}`.toLowerCase().includes(query))) {
            const node = fluxTemplate('flux-voyage', `${item.name || item.session_id} · ${item.state}`); node.addEventListener('click', () => select(item.session_id)); node.toggleAttribute('data-current', item.session_id === selected); node.setAttribute('aria-current', String(item.session_id === selected)); $('voyages').append(node);
        }
    }
    function select(id) {
        if (busy || refreshing) return notice('Wait for the current operation, then switch voyages.');
        selected = id; snapshot = null; incarnation = null; revision = null; messages = []; decisions = []; earliest = 0; stale = true;
        messageFingerprint = ''; decisionFingerprint = ''; outputFingerprint = ''; $('messages').replaceChildren(); $('decisions').replaceChildren(); $('live-output').hidden = true;
        renderVoyages(); controls(); refresh();
    }
    async function refresh() {
        if (!client || refreshing || busy) return;
        refreshing = true; const active = client, mine = generation, id = selected;
        try {
            if (Date.now() - lastCatalogue > 10000) {
                const response = await active.exchange(request('catalogue'));
                if (response.protocol !== 1 || response.error != null || response.outcome_unknown !== false || !Array.isArray(response.result)) throw new Error('Catalogue unavailable.');
                if (mine !== generation) return;
                voyages = response.result; lastCatalogue = Date.now(); renderVoyages();
            }
            if (!id) { notice('Select an existing voyage.'); return; }
            for (const entry of journal.entries().filter(e => e.session_id === id).slice(0, 16)) {
                const response = await active.exchange(request('receipt', {session_id:id,command_id:entry.command_id}));
                if (resolved(response, entry.command_id, id, true)) { journal.settle(entry.command_id); notice(`Receipt ${entry.command_id}: ${response.result.result.status}. This is not a claim that execution completed.`); }
            }
            const envelope = voyageResult(await active.exchange(request('snapshot', {session_id:id})), id);
            const next = envelope.result;
            if (next.session_id !== id || !Number.isSafeInteger(next.revision)) throw new Error('Invalid snapshot identity.');
            const decisionReply = voyageResult(await active.exchange(request('decisions', {session_id:id})), id, envelope.incarnation);
            if (mine !== generation || selected !== id || active !== client) return;
            const changed = next.revision !== revision || envelope.incarnation !== incarnation;
            snapshot = next; incarnation = envelope.incarnation; decisions = Array.isArray(decisionReply.result) ? decisionReply.result : [];
            if (changed) { revision = next.revision; messages = next.messages || []; earliest = next.message_offset || 0; }
            stale = false; lastFresh = Date.now(); $('voyage-title').textContent = clean(next.name || id); state(`Connected · ${next.run?.state || 'idle'}`);
            renderMessages(); renderDecisions(); renderOutput();
        } catch (error) { if (mine === generation) { stale = true; state('Stale · refresh required'); notice(error.message); } }
        finally { if (mine === generation) { refreshing = false; controls(); } }
    }
    function renderMessages() {
        const fingerprint = JSON.stringify(messages); if (fingerprint === messageFingerprint) return; messageFingerprint = fingerprint;
        const scroll = $('conversation'), atBottom = scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 80;
        $('messages').replaceChildren();
        for (const message of messages) {
            if (message.role === 'system') continue;
            const node = fluxTemplate('flux-card');
            node.append(fluxTemplate('flux-heading', `${message.role || 'message'}${message.interrupted_attempt ? ' · interrupted attempt' : ''}`));
            const body = element('div', null, 'text-sm leading-7 text-zinc-700 dark:text-zinc-200 [&_p]:my-3 [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:bg-zinc-100 [&_pre]:p-4 dark:[&_pre]:bg-zinc-900 [&_code]:font-mono [&_a]:underline [&_ul]:list-disc [&_ul]:pl-6 [&_ol]:list-decimal [&_ol]:pl-6 [&_blockquote]:border-l-2 [&_blockquote]:pl-4'); body.innerHTML = markdown(message.content || '');
            for (const link of body.querySelectorAll('a')) { link.rel = 'noopener noreferrer'; link.target = '_blank'; }
            node.append(body);
            if (message.tool_calls?.length || message.tool_output || message.parts?.length) {
                const details = element('details'); details.append(element('summary','Tool / attachment details'), element('pre', JSON.stringify({tool_calls:message.tool_calls,tool_output:message.tool_output,parts:message.parts},null,2))); node.append(details);
            }
            if (message.projection_truncated) node.append(button('Read complete message', () => expand(message.message_index)));
            $('messages').append(node);
        }
        $('earlier').hidden = earliest <= 0;
        if (atBottom) scroll.scrollTop = scroll.scrollHeight;
    }
    async function earlier() {
        if (!actionable() || refreshing) return;
        busy = true; controls(); const active = client, id = selected, rev = revision;
        try {
            const offset = Math.max(0, earliest - 32);
            const page = voyageResult(await active.exchange(request('history',{session_id:id,offset,limit:Math.min(32,earliest-offset),expected_revision:rev})),id,incarnation).result;
            if (page.revision !== rev || page.message_offset !== offset || !Number.isSafeInteger(page.next_offset) || page.next_offset <= offset) throw new Error('History changed; refresh required.');
            messages = [...page.messages, ...messages].filter((v,i,a) => a.findIndex(m => m.message_index === v.message_index) === i).sort((a,b) => a.message_index-b.message_index);
            // Page byte limits may leave a gap. Never label that gap as loaded.
            if (page.next_offset < earliest) {
                let next = page.next_offset;
                while (next < earliest) {
                    const rest = voyageResult(await active.exchange(request('history',{session_id:id,offset:next,limit:earliest-next,expected_revision:rev})),id,incarnation).result;
                    if (rest.revision !== rev || !Number.isSafeInteger(rest.next_offset) || rest.next_offset <= next) throw new Error('History changed.');
                    messages.push(...rest.messages); next = rest.next_offset;
                }
                messages.sort((a,b) => a.message_index-b.message_index);
            }
            earliest = offset; renderMessages();
        } catch (error) { stale = true; notice(error.message); } finally { busy = false; controls(); }
    }
    async function expand(index) {
        if (!actionable() || refreshing) return;
        busy = true; controls();
        try {
            let offset = 0, text = '', page;
            do {
                page = voyageResult(await client.exchange(request('message_chunk',{session_id:selected,index,offset,limit:65536,expected_revision:revision})),selected,incarnation).result;
                if (page.total_bytes > 4*1024*1024) throw new Error('Message exceeds this browser client’s 4 MiB expansion limit. Use native Helm for the complete message.');
                if (page.has_more && page.next_offset <= offset) throw new Error('Invalid message continuation.');
                text += page.data; offset = page.next_offset;
            } while (page.has_more);
            messages = messages.map(m => m.message_index === index ? {...JSON.parse(text),message_index:index,projection_truncated:false} : m); renderMessages();
        } catch (error) { notice(error.message); } finally { busy = false; controls(); }
    }
    function renderOutput() {
        const run = snapshot?.run;
        const fingerprint = JSON.stringify([run?.run_id, run?.live_text, run?.partial_text, run?.stream_reconciled, run?.live_text_offset]);
        if (fingerprint === outputFingerprint) return; outputFingerprint = fingerprint; outputOffset = null;
        $('live-output').hidden = !run || (!run.live_text && !run.partial_text && !run.live_text_truncated);
        if (!run) return;
        $('live-output pre').textContent = clean(run.stream_reconciled ? run.live_text : run.partial_text);
        $('live-output h2').textContent = run.stream_reconciled ? 'Live output · provisional, not yet canonical' : 'Run output · unreconciled, may overlap history';
        const text = run.stream_reconciled ? run.live_text || '' : run.partial_text || '';
        outputOffset = (run.stream_reconciled ? run.live_text_offset : 0) + new TextEncoder().encode(text).length;
        $('more-output').hidden = !(run.stream_reconciled ? run.live_text_truncated : run.partial_text_truncated);
    }
    async function moreOutput() {
        if (!actionable() || refreshing) return;
        busy = true; controls();
        try {
            const page = voyageResult(await client.exchange(request('run_output',{session_id:selected,run_id:snapshot.run.run_id,offset:outputOffset,limit:65536})),selected,incarnation).result;
            if (page.run_id !== snapshot.run.run_id || page.offset !== outputOffset || (page.has_more && page.next_offset <= outputOffset)) throw new Error('Output identity changed.');
            outputOffset = page.next_offset;
            $('live-output pre').append(document.createTextNode(clean(page.data))); $('more-output').hidden = !page.has_more;
        } catch (error) { notice(error.message); } finally { busy = false; controls(); }
    }
    function renderDecisions() {
        const fingerprint = JSON.stringify(decisions); if (fingerprint === decisionFingerprint) return; decisionFingerprint = fingerprint; $('decisions').replaceChildren();
        for (const decision of decisions) {
            if (decision.expires_at_ms <= Date.now() || decision.incarnation !== incarnation || decision.run_id !== snapshot?.run?.run_id) continue;
            const card = fluxTemplate('flux-decision'); card.dataset.expires = decision.expires_at_ms; const content = card.querySelector('[data-slot=content]'); const value = decision.request;
            if (value?.kind === 'approval') {
                content.append(fluxTemplate('flux-heading','Approval requested'),element('pre',JSON.stringify(value.approval,null,2)));
                content.append(button('Approve',() => act('respond',decision,'approved')),button('Deny',() => act('respond',decision,'denied')));
            } else if (value?.kind === 'question') {
                content.append(fluxTemplate('flux-heading',value.question.question),fluxTemplate('flux-text','Your answer becomes conversation history. Never enter a password or secret.'));
                value.question.options.forEach((answer,index) => content.append(button(answer,() => act('respond',decision,{status:'selected',index,answer}))));
                const field = fluxTemplate('flux-answer'); const input = field.matches('input') ? field : field.querySelector('input'); content.append(field,button('Send custom answer',() => { if (input.value.trim()) act('respond',decision,{status:'custom',answer:input.value}); }),button('Skip question',() => act('respond',decision,{status:'cancelled'})));
            } else { content.append(fluxTemplate('flux-text','This decision needs a native Helm client. No approval sent.')); }
            $('decisions').append(card);
        }
    }
    async function act(op, decision = null, answer = null) {
        if (!actionable() || refreshing) return notice('Wait for a fresh connected snapshot before acting.');
        const text = $('prompt').value;
        if (['submit','steer'].includes(op) && (!text.trim() || new TextEncoder().encode(text).length > 65536)) return notice('Message must contain 1–65536 UTF-8 bytes.');
        if (decision && (decision.expires_at_ms <= Date.now() || decision.incarnation !== incarnation || decision.run_id !== snapshot.run?.run_id)) return notice('Decision expired. Refresh before responding.');
        busy = true; controls();
        const fields = ['submit','steer'].includes(op) ? {prompt:text} : decision ? {decision_id:decision.decision_id,response:answer,expires_at_ms:Math.min(Date.now()+60000,decision.expires_at_ms)} : {};
        let command;
        try {
            command = mutation(op,snapshot,incarnation,fields); journal.prepare(command.command);
            const response = await client.exchange(command);
            if (resolved(response,command.command.command_id,selected)) journal.settle(command.command.command_id);
            if (response.error != null) notice('Vessel refused the action. Refresh before deciding what to do next.');
            else if (response.outcome_unknown || !resolved(response,command.command.command_id,selected)) notice('Outcome uncertain. Checking receipts only; the action will not be resent.');
            else { notice(`Acknowledged ${command.command.command_id}; execution may still be pending.`); if (['submit','steer'].includes(op) && $('prompt').value === text) $('prompt').value = ''; }
        } catch { notice('Action not confirmed. Any recorded command remains pending; reconnect checks receipts without resending.'); }
        finally { busy = false; stale = true; controls(); refresh(); }
    }
    $('composer').addEventListener('submit', event => { event.preventDefault(); act(running() ? 'steer' : 'submit'); });
    $('prompt').addEventListener('keydown', event => { if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.isComposing) { event.preventDefault(); act(running() ? 'steer' : 'submit'); } });
    $('cancel').addEventListener('click',() => act('cancel'));
    $('earlier').addEventListener('click',earlier); $('more-output').addEventListener('click',moreOutput);
    $('voyage-search').addEventListener('input',renderVoyages);
    $('reconnect').addEventListener('click',() => { stopped = false; connect(); });
    $('vessel').addEventListener('change',() => { if (busy) return; selected=null;snapshot=null;messages=[];decisions=[];journal=null;voyages=[];revision=null;incarnation=null;messageFingerprint='';decisionFingerprint='';outputFingerprint='';$('live-output').hidden=true;$('voyage-title').textContent='Choose a voyage';renderVoyages();$('messages').replaceChildren();$('decisions').replaceChildren();connect(); });
    window.addEventListener('storage', () => controls());
    const timer = setInterval(() => { controls(); if (!document.hidden) refresh(); },1000);
    window.addEventListener('pagehide',() => { stopped=true;generation++;clearInterval(timer);clearInterval(renewal);clearTimeout(reconnectTimer);client?.close(); });
    document.addEventListener('visibilitychange',() => { if (!document.hidden) { stale=true;controls();refresh(); } });
    connect();
}
const root = typeof document !== 'undefined' && document.querySelector('#helm-client');
if (root) mount(root);
