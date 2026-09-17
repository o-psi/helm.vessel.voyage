import {sidebarActions} from './sidebar-actions.js';
import {conversationScroll} from './conversation-scroll.js';
import {ConversationStream, renderPreviews} from './conversation-stream.js';
import {connectionDiagnostic as log} from './connection-diagnostics.js';
import {actionDescription, actionStatus, actionDuration} from './tool-presentation.js';
import {setReasoning, reasoningValue} from './inference-controls.js';
import {marked} from 'marked';
import DOMPurify from 'dompurify';
import {request, voyageResult, mutation, resolved, receiptStatus} from './vessel-client.js';
import {VesselFleet} from './vessel-fleet.js';
import {composer} from './composer.js';
import {imageBytes} from './attachments.js';
import {voyageSettings} from './voyage-settings.js';

const clean = value => String(value ?? '').replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/g, '');
export function markdown(text) {
    // No remote images, raw HTML controls, embedded content or executable links.
    return DOMPurify.sanitize(marked.parse(clean(text)), {ALLOWED_TAGS: ['p','br','strong','em','del','code','pre','blockquote','ul','ol','li','h1','h2','h3','h4','hr','a','table','thead','tbody','tr','th','td'], ALLOWED_ATTR: ['href','title'], ALLOW_DATA_ATTR: false});
}
function fluxTemplate(id, label) {
    const template = document.getElementById(id);
    if (!template) throw new Error(`Missing Flux template: ${id}`);
    const node = template.content.firstElementChild.cloneNode(true);
    if (label != null) node.querySelector('[data-label]').textContent = clean(label);
    return node;
}
function button(label, action) { const node = fluxTemplate('flux-action', label); node.type = 'button'; node.addEventListener('click', action); return node; }

// Sanitization happens before presentation; untrusted HTML never creates Flux controls.
export function messageContent(text) {
    const body = document.createElement('div');
    body.className = 'space-y-4 text-[15px] leading-7 [text-wrap:pretty] [&_p]:leading-7 [&_li]:my-1 [&_pre]:text-sm [&_table]:text-sm';
    body.innerHTML = markdown(text);
    for (const node of [...body.querySelectorAll('p,h1,h2,h3,h4,a,pre,blockquote,hr,table')].reverse()) {
        let replacement, target;
        if (node.matches('table')) {
            replacement = fluxTemplate('flux-table');
            const head = replacement.querySelector('thead tr'), rows = replacement.querySelector('tbody');
            const column = head.firstElementChild.cloneNode(true), row = rows.firstElementChild.cloneNode(true), cell = row.firstElementChild.cloneNode(true);
            head.replaceChildren(); rows.replaceChildren();
            for (const source of node.querySelectorAll('tr')) {
                const isHead = source.parentElement.tagName === 'THEAD';
                const dest = isHead ? head : row.cloneNode(false);
                for (const sourceCell of source.children) {
                    const destCell = (isHead ? column : cell).cloneNode(true);
                    (destCell.querySelector('[data-cell]') || destCell).replaceChildren(...sourceCell.childNodes);
                    dest.append(destCell);
                }
                if (!isHead) rows.append(dest);
            }
        } else {
            const template = node.matches('p') ? 'flux-text' : node.matches('a') ? 'flux-link' : node.matches('pre') ? 'flux-code' : node.matches('blockquote') ? 'flux-quote' : node.matches('hr') ? 'flux-separator' : `flux-${node.tagName.toLowerCase()}`;
            replacement = fluxTemplate(template);
            target = replacement.querySelector('[data-label],[data-code],[data-quote]') || replacement;
            target.replaceChildren(...node.childNodes);
            if (node.matches('a')) {
                const href = node.getAttribute('href');
                if (href) replacement.setAttribute('href', href); else replacement.removeAttribute('href');
                if (node.title) replacement.title = node.title;
            }
        }
        node.replaceWith(replacement);
    }
    // Lists and code are semantic message content, not application controls.
    for (const list of body.querySelectorAll('ul,ol')) list.className = list.matches('ul') ? 'list-disc ps-6' : 'list-decimal ps-6';
    return body;
}

export function mount(root) {
    const $ = id => root.querySelector(`#${id}`);
    const scrolling = conversationScroll($('conversation'), fluxTemplate('flux-action', 'Jump to latest ↓'));
    let client, journal, selectedVessel = null, selected = null, snapshot = null, incarnation = null, generation = 0, stale = true, busy = false, refreshing = false;
    let messages = [], decisions = [], earliest = 0, revision = null;
    let composition, newChatSending = false;
    const admittedDrafts = new Map();
    const sentDraft = command_id => admittedDrafts.get(command_id);
    function forgetSent(command_id) { admittedDrafts.delete(command_id); }
    const stream = new ConversationStream();
    let unsubscribe = null, streamEpoch = 0, refreshQueued = false;
    function stopStream() { streamEpoch++; const stop = unsubscribe; unsubscribe = null; try { stop?.(); } catch { /* Closed sockets need no unsubscribe acknowledgement. */ } }
    function subscribe() {
        if (unsubscribe || !client || !stream.valid || typeof client.subscribe !== 'function') return;
        const epoch = streamEpoch, active = client;
        try {
            unsubscribe = active.subscribe(selected, incarnation, stream.cursor, event => {
                if (epoch !== streamEpoch || active !== client) return;
                const action = stream.accept(event);
                if (action === 'duplicate') return;
                if (action === 'resync') { stopStream(); stale = true; controls(); }
                // Streaming invalidations refresh promptly without disabling a
                // click against a still-fresh revision on every token.
                if (refreshing || busy) refreshQueued = true; else refresh();
            });
        } catch { stopStream(); stale = true; }
    }
    let settings;
    let accountLabelKey = '', accountLabel = 'Account', accountLabelClient;
    const fleet = new VesselFleet(JSON.parse(root.dataset.vessels || '[]'), {tenantId:root.dataset.tenantId, ticket, changed:connectionsChanged});
    let voyageFingerprint = '', messageFingerprint = '', decisionFingerprint = '', lastFresh = 0, outputFingerprint = '', outputOffset = null;
    const state = text => { $('connection-state').textContent = text; };
    const notice = text => { $('notice').textContent = clean(text); $('notice-panel').hidden = !text; };
    const running = () => ['running','starting','cancelling'].includes(snapshot?.run?.state);
    const pending = () => journal?.entries().filter(e => e.session_id === selected) || [];
    const actionable = () => client && snapshot && !stale && !busy && Date.now() - lastFresh < 35000 && !pending().length;
    function controls() {
        try {
            const enabled = actionable();
            const newChatSettings = !selected && client && !busy && !newChatSending;
            const reviewed = !selected ? settings?.configuration() : null;
            $('reconnect').disabled = busy; $('prompt').disabled = busy || newChatSending || !selectedVessel; $('send').disabled = newChatSending || (!enabled && !(!selected && composition?.active?.document?.target?.type === 'new_chat' && !busy && client)); $('cancel').disabled = !enabled || !running();
            $('cancel').hidden = !running();
            $('send').setAttribute('aria-label', running() ? 'Steer run' : 'Send');
            $('send').title = running() ? 'Steer run · Enter' : 'Send · Enter';
            $('change-inference').disabled = !newChatSettings && (!enabled || running());
            $('change-account').disabled = !newChatSettings && (!enabled || running());
            $('change-service').disabled = !newChatSettings && (!enabled || running());
            $('change-location').hidden = Boolean(selected);
            $('change-location').disabled = !newChatSettings;
            $('composer-location').textContent = reviewed ? `${fleet.connections.get(selectedVessel)?.name} · ${reviewed.workspace}` : 'Vessel / Workspace';
            $('change-reasoning').disabled = !reviewed && (!enabled || running() || !snapshot?.inference?.account);
            $('change-access').disabled = !enabled && !newChatSettings;
            $('composer-access').textContent = ({'read-only':'Read only',approval:'Approval',unrestricted:'Full access'})[!selected ? composition?.active?.access : snapshot?.access] || (!selected ? 'Vessel default' : 'Access unknown');
            $('access-mode').disabled = !enabled && !newChatSettings;
            $('access-mode').value = !selected ? composition?.active?.access || '' : ['read-only','approval','unrestricted'].includes(snapshot?.access) ? snapshot.access : '';
            $('new-voyage').disabled = busy || newChatSending;
            $('composer-model').textContent = clean(reviewed?.settings.model || snapshot?.inference?.model || snapshot?.model || 'Account & model');
            $('composer-reasoning').textContent = clean(reviewed?.settings.reasoning_effort || snapshot?.inference?.reasoning_effort || 'Default');
            $('composer-service').textContent = clean(reviewed ? reviewed.settings.service_tier || 'Default tier' : snapshot?.inference ? snapshot.inference.service_tier || 'Default tier' : 'Service');
            $('composer-account').textContent = reviewed?.accountLabel || accountLabel;
            $('inference-summary').textContent = clean([snapshot?.inference?.provider, snapshot?.inference?.model || snapshot?.model].filter(Boolean).join(' · '));
            $('pending').replaceChildren();
            for (const entry of pending()) $('pending').append(fluxTemplate('flux-text', `${entry.op} · ${entry.command_id} · outcome not confirmed. Receipt checks only; never automatically resent.`));
            for (const node of $('decisions').querySelectorAll('button,input')) node.disabled = !enabled || Number(node.closest('.decision').dataset.expires) <= Date.now();
        } catch { stale = true; $('send').disabled = true; $('cancel').disabled = true; notice('Browser command journal unavailable. Sending is disabled to prevent uncertain duplicate work.'); }
    }
    async function ticket(alias) {
        const response = await fetch(root.dataset.ticketUrl, {method: 'POST', credentials: 'same-origin', headers: {'Content-Type':'application/json','Accept':'application/json','X-CSRF-TOKEN':document.querySelector('meta[name="csrf-token"]').content}, body: JSON.stringify({vessel:alias})});
        if (!response.ok) { const error = new Error([401,403,419].includes(response.status) ? 'Sign-in required' : response.status === 404 ? 'Connection removed' : 'Authorization unavailable'); error.permanent = [401,403,404,419].includes(response.status); throw error; }
        return await response.json();
    }
    function connectionsChanged() {
        const connection = fleet.connections.get(selectedVessel);
        if (client !== connection?.client) {
            stopStream(); generation++; client = connection?.client; journal = connection?.journal;
            stale = true; refreshing = false;
        }
        const connections = [...fleet.connections.values()];
        if (!selectedVessel && connections.some(c=>c.client)) { const first=connections.find(c=>c.client); selectedVessel=first.id; client=first.client; journal=first.journal; composition?.select().catch(error=>notice(error.message)); }
        const connected = connections.filter(c => c.client).length;
        $('fleet-state').textContent = connections.length ? `${connected} of ${connections.length} Vessels connected` : 'No Vessels connected';
        $('vessel-statuses').replaceChildren();
        for (const c of connections.filter(c => c.status !== 'Connected')) {
            $('vessel-statuses').append(fluxTemplate('flux-text', `${c.name} · ${c.status}`));
        }
        if (!selected) state(connected ? 'Ready' : connections.length ? 'Connecting…' : 'No Vessels connected');
        else if (!client) state(`${connection?.name || 'Vessel'} · ${connection?.status || 'Unavailable'}`);
        else if (stale) state('Loading voyage…');
        renderVoyages(); controls(); settings?.renderPending();
        if (selected && client && stale) refresh();
    }
    const sidebar = sidebarActions(root, {changed: () => { voyageFingerprint = null; fleet.poll(); refresh(); }});
    function renderVoyages() {
        const query = $('voyage-search').value.toLowerCase();
        const voyages = [...fleet.connections.values()].flatMap(connection => connection.voyages.map(v => ({...v, connection})));
        const fingerprint = JSON.stringify([voyages.map(v => [v.session_id,v.name,v.state,v.connection.id,Boolean(v.connection.client)]), query, selectedVessel, selected]);
        if (fingerprint === voyageFingerprint) return;
        voyageFingerprint = fingerprint; $('voyages').replaceChildren();
        const filtered = voyages.filter(v => `${v.name || ''} ${v.session_id} ${v.connection.name}`.toLowerCase().includes(query));
        $('voyage-empty').hidden = filtered.length > 0;
        $('voyage-empty').textContent = query ? 'No voyages match your search.' : 'Voyages from your connected Vessels appear here.';
        for (const item of filtered) {
            const label = clean(item.name || item.session_id), connection = item.connection;
            const description = clean(`${label} · ${connection.name} · ${connection.client ? item.state : 'offline'}`);
            const node = fluxTemplate('flux-voyage', label), control = node.querySelector('button');
            node.querySelector('[data-vessel-label]').textContent = clean(connection.name);
            control.addEventListener('click', () => select(connection.id, item.session_id, label));
            const current = item.session_id === selected && connection.id === selectedVessel;
            control.toggleAttribute('data-current', current); control.setAttribute('aria-current', String(current));
            control.setAttribute('aria-label', description); control.title = description;
            const tooltip = node.querySelector('[data-flux-tooltip-content]');
            if (tooltip) tooltip.textContent = description;
            sidebar.bind(node, connection, item);
            $('voyages').append(node);
        }
    }
    function select(vessel, id, title, retainDraft = false) {
        sidebar.invalidate();
        if (busy) return notice('Wait for the current operation, then switch voyages.');
        root.querySelector('[data-flux-sidebar-on-mobile]:not([data-flux-sidebar-collapsed-mobile]) [data-flux-sidebar-collapse] button')?.click();

        composition?.remember();
        stopStream(); refreshQueued = false;
        scrolling.reset();
        notice(''); generation++; refreshing = false; selectedVessel = vessel; selected = id;
        client = fleet.connections.get(vessel).client; journal = fleet.connections.get(vessel).journal;
        if (!retainDraft) $('prompt').value = '';
        !retainDraft && composition?.select().catch(error => notice(error.message));
        $('voyage-title').textContent = title;
        $('voyage-vessel').textContent = clean(fleet.connections.get(vessel).name); $('voyage-vessel').hidden = false;
        accountLabelKey = ''; accountLabel = 'Account';
        snapshot = null; incarnation = null; revision = null; messages = []; decisions = []; earliest = 0; stale = true;
        messageFingerprint = ''; decisionFingerprint = ''; outputFingerprint = ''; $('messages').replaceChildren(); $('decisions').replaceChildren(); renderPreviews($('live-previews'), null); $('live-output').hidden = true; $('earlier').hidden = true;
        $('conversation-empty').hidden = false;
        $('conversation-empty').textContent = !selected ? 'Write your first message below. Your first Send creates the voyage.' : client ? 'Loading conversation…' : 'This Vessel is unavailable. Its voyages remain listed while the connection recovers.';
        connectionsChanged(); refresh();
    }
    async function refresh() {
        if (!client || !selected || refreshing || busy) return;
        refreshing = true; const active = client, activeJournal = journal, mine = generation, id = selected;
        try {
            if (!id) return;
            for (const entry of activeJournal.entries().filter(e => e.session_id === id && !e.sidebar_action).slice(0, 16)) {
                const response = await active.exchange(request('receipt', {session_id:id,command_id:entry.command_id}));
                if (mine !== generation || active !== client) return;
                if (resolved(response, entry.command_id, id, true)) { const sent = sentDraft(entry.command_id); if(sent && ['accepted','queued','applied'].includes(receiptStatus(response))){try {await composition.admitted(sent);forgetSent(entry.command_id);}catch(error){notice(`Message admitted; composer cleanup failed: ${error.message}`);continue;}} activeJournal.settle(entry.command_id); notice(`Receipt ${entry.command_id}: ${receiptStatus(response)}. This is not a claim that execution completed.`); }
            }
            const envelope = voyageResult(await active.exchange(request('snapshot', {session_id:id})), id);
            const next = envelope.result;
            if (next.session_id !== id || !Number.isSafeInteger(next.revision)) throw new Error('Invalid snapshot identity.');
            const decisionReply = voyageResult(await active.exchange(request('decisions', {session_id:id})), id, envelope.incarnation);
            if (mine !== generation || selected !== id || active !== client) return;
            const changed = next.revision !== revision || envelope.incarnation !== incarnation;
            snapshot = next; incarnation = envelope.incarnation;
            stream.seed(next, incarnation);
            updateAccountLabel(); decisions = Array.isArray(decisionReply.result) ? decisionReply.result : [];
            if (changed) { revision = next.revision; messages = next.messages || []; earliest = next.message_offset || 0; }
            stale = false; lastFresh = Date.now(); log('snapshot_fresh',{generation:mine}); $('voyage-title').textContent = clean(next.name || id); state(`Connected · ${next.run?.state || 'idle'}`);
            $('conversation-empty').hidden = messages.length > 0; $('conversation-empty').textContent = 'No messages yet. Send a message to begin.';
            renderMessages(); renderDecisions(); renderOutput(); subscribe();
        } catch (error) { if (mine === generation) { stale = true; state('Stale · refresh required'); $('conversation-empty').hidden = messages.length > 0; $('conversation-empty').textContent = 'Conversation unavailable. Reconnecting to its Vessel…'; notice(error.message); } }
        finally { if (mine === generation) { refreshing = false; controls(); if (refreshQueued) { refreshQueued = false; queueMicrotask(refresh); } } }
    }
    async function updateAccountLabel() {
        const binding = snapshot?.inference?.account;
        const key = JSON.stringify([selectedVessel,selected,incarnation,binding]);
        if (key === accountLabelKey && accountLabelClient === client) return;
        accountLabelKey = key; accountLabelClient = client;
        accountLabel = binding ? 'Loading account…' : 'No account';
        if (!binding) return;
        const active = client, id = selected, expectedIncarnation = incarnation;
        const current = () => accountLabelKey === key && client === active;
        try {
            const read = async (op, fields) => {
                const reply = await active.exchange(request(op,fields));
                if (reply.protocol !== 1 || reply.error != null || reply.outcome_unknown !== false) throw new Error('Account unavailable');
                return reply.result;
            };
            const process = await read('inspect',{session_id:id});
            if (!current()) return;
            if (process.session_id !== id || process.incarnation !== expectedIncarnation || !process.workspace) throw new Error('Account context changed');
            const catalogue = await read('accounts',{workspace:process.workspace,transport:null});
            if (!current()) return;
            const account = catalogue.accounts?.find(a => a.id === binding.account_id && a.connection_id === binding.connection_id && a.identity_generation === binding.identity_generation);
            const connection = catalogue.connections?.find(c => c.id === binding.connection_id && c.revision === binding.connection_revision && c.transports?.includes(binding.transport));
            accountLabel = account && connection ? clean(account.label) || 'Unnamed account' : 'Account unavailable';
        } catch { if (current()) accountLabel = 'Account unavailable'; }
        if (current()) controls();
    }
    function renderMessages() {
        const fingerprint = JSON.stringify(messages); if (fingerprint === messageFingerprint) return; messageFingerprint = fingerprint;
        const opened = new Set([...$('messages').querySelectorAll('[data-tool-entry][open]')].map(node => node.dataset.key));
        const olderOpen = new Set([...$('messages').querySelectorAll('[data-tool-group][data-older-open]')].map(n=>n.dataset.key));
        const focused = document.activeElement?.closest('[data-tool-entry]')?.dataset.key;
        $('messages').replaceChildren();
        let group = null, groupMessages = [];
        const calls = new Map(messages.flatMap(m => (m.tool_calls || []).map(call => [call.id,call])));
        const stamp = (element, message) => {
            const date = message.created_at ? new Date(message.created_at) : null;
            if (!date || !Number.isFinite(date.getTime())) { element.hidden = true; return; }
            element.dateTime = date.toISOString(); element.title = date.toLocaleString();
            element.textContent = date.toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'});
        };
        const renderOne = (message, container, tool = false) => {
            const node = fluxTemplate(tool ? 'flux-card' : message.role === 'user' ? 'thread-user' : 'thread-assistant');
            if (tool) node.append(fluxTemplate('flux-text', message.role === 'tool' ? 'Tool result' : 'Tool request'));
            const interrupted = node.querySelector('[data-interrupted]');
            if (interrupted) interrupted.hidden = !message.interrupted_attempt;
            if (!message.parts?.length) node.append(messageContent(message.content || ''));
            for (const part of message.parts || []) {
                if (part.type === 'text') { node.append(messageContent(part.text)); continue; }
                if (part.type !== 'image') continue;
                const img = document.createElement('img'); img.alt = clean(part.attachment.name); img.className = 'max-w-full max-h-72 rounded-lg object-contain';
                const label = document.createElement('span'); label.textContent = 'Loading authorized picture…'; node.append(img,label);
                const activeClient = client, session_id = selected;
                imageBytes(activeClient,part.attachment,{session_id}).then(blob => {
                    if (!img.isConnected || selected !== session_id || client !== activeClient) return;
                    const url = URL.createObjectURL(blob); img.onload = img.onerror = () => URL.revokeObjectURL(url); img.src = url; label.remove();
                }).catch(() => { label.textContent = 'Picture unavailable. Reconnect with history access to retry.'; });
            }
            if (message.tool_calls?.length || message.tool_output || message.parts?.length) {
                const details = fluxTemplate('flux-details');
                details.querySelector('button').addEventListener('click', () => {
                    $('message-details-content').textContent = clean(JSON.stringify({tool_calls:message.tool_calls,tool_output:message.tool_output,parts:message.parts},null,2));
                });
                node.append(details);
            }
            if (message.projection_truncated) node.append(button('Read complete message', () => expand(message.message_index)));
            if (!tool) { const time = document.createElement('time'); time.className = 'block text-end text-[11px] text-zinc-400'; stamp(time,message); node.append(time); }
            container.append(node);
        };
        for (const message of messages) {
            if (message.role === 'system') continue;
            const tool = message.role === 'tool' || message.role === 'function' || (message.tool_calls?.length && !String(message.content || '').trim());
            if (!tool) { group = null; renderOne(message,$('messages')); if (!message.tool_calls?.length) continue; }
            if ((message.role === 'tool' || message.role === 'function') && calls.has(message.tool_call_id)) continue;
            if (!group) {
                group = fluxTemplate('thread-tools'); group.dataset.key = String(message.message_index);
                groupMessages = []; const current = group;
                current.querySelector('[data-expand-tools]').addEventListener('click', () => {
                    current.toggleAttribute('data-older-open'); current.updateOlder();
                });
                $('messages').append(current);
            }
            groupMessages.push(message);
            const entries = message.tool_calls?.length ? message.tool_calls.map(call => ({call, request:true})) : [{call:calls.get(message.tool_call_id),request:false}];
            for (const [index,item] of entries.entries()) {
                const entry = fluxTemplate('thread-tool-entry'); entry.dataset.key = item.call?.id || `${message.message_index}:${index}`;
                const result = item.request ? messages.find(m => m.tool_call_id === item.call.id) : message;
                entry.dataset.finished = String(Boolean(result));
                const active = running() && Number.isSafeInteger(snapshot?.run?.message_start) && message.message_index >= snapshot.run.message_start;
                const name = clean(item.call?.function?.name || item.call?.name || message.name || 'Tool');
                const status = actionStatus(item.call || {},result,active,decisions.length > 0);
                entry.dataset.failed = String(result && !['Done','Received'].includes(status));
                entry.querySelector('[data-entry-label]').textContent = clean([status,actionDuration(result),item.call ? actionDescription(item.call) : name].filter(Boolean).join(' · '));
                entry.querySelector('[data-entry-time]').hidden = true;
                let rendered = false;
                entry.renderTool = () => {
                    if (rendered || !entry.open) return; rendered = true;
                    const body = entry.querySelector('[data-entry-body]');
                    if (item.request) {
                        const code = fluxTemplate('flux-code');
                        code.querySelector('[data-code]').textContent = clean(JSON.stringify(item.call.function?.arguments ?? item.call.arguments ?? {},null,2)); body.append(code);
                        if (message.projection_truncated) body.append(button('Read complete message',() => expand(message.message_index)));
                    }
                    if (result) renderOne(result,body,true);
                };
                entry.addEventListener('toggle',entry.renderTool);
                entry.open = opened.has(entry.dataset.key); entry.renderTool();
                group.querySelector('[data-tool-body]').append(entry);
                if (focused === entry.dataset.key) entry.querySelector('summary').focus({preventScroll:true});
            }
            const count = group.querySelectorAll('[data-tool-entry]').length;
            group.querySelector('[data-tool-label]').textContent = `${count} ${count === 1 ? 'action' : 'actions'}`;
        }
        for (const group of $('messages').querySelectorAll('[data-tool-group]')) {
            if (olderOpen.has(group.dataset.key)) group.setAttribute('data-older-open','');
            group.updateOlder = () => {
                const entries = [...group.querySelectorAll('[data-tool-entry]')], older = entries.slice(0,-3);
                const expanded = group.hasAttribute('data-older-open');
                older.forEach(entry => { entry.hidden = !expanded; });
                const button = group.querySelector('[data-expand-tools]'); button.hidden = older.length === 0;
                button.textContent = expanded ? 'Hide older actions' : 'Show older actions';
                if (older.length) group.querySelector('[data-tool-label]').textContent = `${older.length} older actions · ${older.filter(e=>e.dataset.finished==='true').length}/${older.length} finished${older.some(e=>e.dataset.failed==='true') ? ' · failures or incomplete outcomes' : ''}`;
            };
            group.updateOlder();
        }
        $('earlier').hidden = earliest <= 0;
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
            earliest = offset; scrolling.preservePrepend(renderMessages);
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
        renderPreviews($('live-previews'), run, messages);
        const fingerprint = JSON.stringify([run?.run_id, run?.live_text, run?.partial_text, run?.stream_reconciled, run?.live_text_offset, run?.live_text_truncated, run?.partial_text_truncated]);
        if (fingerprint === outputFingerprint) return; outputFingerprint = fingerprint; outputOffset = null;
        const text = run?.stream_reconciled ? run.live_text || '' : run?.partial_text || '';
        const hasMore = run?.stream_reconciled ? run.live_text_truncated : run?.partial_text_truncated;
        $('live-output').hidden = !text && !hasMore;
        if (!run) return;
        $('output-text').textContent = clean(run.stream_reconciled ? run.live_text : run.partial_text);
        $('output-title').textContent = run.stream_reconciled ? 'Live output · provisional, not yet canonical' : 'Run output · unreconciled, may overlap history';
        outputOffset = (run.stream_reconciled ? run.live_text_offset : 0) + new TextEncoder().encode(text).length;
        $('more-output').hidden = !hasMore;
    }
    async function moreOutput() {
        if (!actionable() || refreshing) return;
        busy = true; controls();
        try {
            const page = voyageResult(await client.exchange(request('run_output',{session_id:selected,run_id:snapshot.run.run_id,offset:outputOffset,limit:65536})),selected,incarnation).result;
            if (page.run_id !== snapshot.run.run_id || page.offset !== outputOffset || (page.has_more && page.next_offset <= outputOffset)) throw new Error('Output identity changed.');
            outputOffset = page.next_offset;
            $('output-text').append(document.createTextNode(clean(page.data))); $('more-output').hidden = !page.has_more;
        } catch (error) { notice(error.message); } finally { busy = false; controls(); }
    }
    function renderDecisions() {
        const fingerprint = JSON.stringify(decisions); if (fingerprint === decisionFingerprint) return; decisionFingerprint = fingerprint; $('decisions').replaceChildren();
        for (const decision of decisions) {
            if (decision.expires_at_ms <= Date.now() || decision.incarnation !== incarnation || decision.run_id !== snapshot?.run?.run_id) continue;
            const card = fluxTemplate('flux-decision'); card.dataset.expires = decision.expires_at_ms; const heading = card.querySelector('[data-decision-heading]'), content = card.querySelector('[data-decision-text]'), actions = card.querySelector('[data-decision-actions]'); const value = decision.request;
            if (value?.kind === 'approval') {
                heading.textContent = 'Approval requested'; content.textContent = clean(JSON.stringify(value.approval,null,2));
                actions.append(button('Approve',() => act('respond',decision,'approved')),button('Deny',() => act('respond',decision,'denied')));
            } else if (value?.kind === 'question') {
                heading.textContent = clean(value.question.question); content.textContent = 'Your answer becomes conversation history. Never enter a password or secret.';
                value.question.options.forEach((answer,index) => actions.append(button(answer,() => act('respond',decision,{status:'selected',index,answer}))));
                const field = fluxTemplate('flux-answer'); const input = field.matches('input') ? field : field.querySelector('input'); actions.append(field,button('Send custom answer',() => { if (input.value.trim()) act('respond',decision,{status:'custom',answer:input.value}); }),button('Skip question',() => act('respond',decision,{status:'cancelled'})));
            } else { content.textContent = 'This decision needs a native Helm client. No approval sent.'; }
            $('decisions').append(card);
        }
    }
    function captureDraft() {
        const origin = composition.capture();
        return origin?.key && composition.active ? {...origin,workspace:composition.active.document.target.workspace,target:structuredClone(composition.active.document.target)} : null;
    }
    async function submitWithAccess() {
        const wanted = composition?.active?.access;
        if (wanted && wanted !== snapshot?.access) {
            const identity = {vessel:selectedVessel,session:selected,incarnation};
            if (running() || !await act('set_access',null,null,{access:wanted})) return notice('Access change not confirmed. Message retained; check the voyage before sending.');
            const deadline = Date.now()+35000;
            while (!actionable() && Date.now()<deadline) await new Promise(resolve=>setTimeout(resolve,25));
            if (identity.vessel !== selectedVessel || identity.session !== selected || identity.incarnation !== incarnation || snapshot?.access !== wanted || !actionable()) return notice('Access change not confirmed. Message retained.');
        }
        if (composition?.active) delete composition.active.access;
        return act(running() ? 'steer' : 'submit');
    }
    async function send() {
        if (newChatSending) return;
        if (selected || composition?.active?.document?.target?.type !== 'new_chat') return submitWithAccess();
        if (!$('prompt').value.trim() && !composition.active.document.parts.some(p=>p.type==='image')) return notice('Write a message before sending.');
        if (!settings.configuration()) { settings.review(); return notice('Review the account and model on this device before sending this new chat.'); }
        newChatSending = true; controls();
        try {
            const sent = await composition.beforeSend();
            const origin = captureDraft();
            const process = await settings.startDraft(origin);
            // Creation and submission have separate immutable receipts. Do not submit
            // until the exact created process has a fresh, actionable owner snapshot.
            const deadline = Date.now()+35000;
            while (selected === process.session_id && selectedVessel === origin.vessel && !actionable() && Date.now()<deadline) await new Promise(resolve=>setTimeout(resolve,25));
            if (selected !== process.session_id || selectedVessel !== origin.vessel || incarnation !== process.incarnation || !actionable()) throw new Error('Chat created; message retained. Wait for its connected snapshot before sending.');
            if (composition.active !== sent.draft) throw new Error('Draft changed; review the composer before sending.');
            await submitWithAccess();
        } catch (error) { notice(error.message); }
        finally { newChatSending=false; controls(); }
    }
    async function act(op, decision = null, answer = null, extra = {}) {
        if (!actionable()) {
            log('action_blocked',{op,connected:Boolean(client),stale,busy,refreshing,fresh_age_ms:Date.now()-lastFresh,journal_pending:pending().length});
            return notice('Wait for a fresh connected snapshot before acting.');
        }
        // A background poll must not reject a click against a still-fresh snapshot.
        // Invalidate its read results before dispatch; owner revision checks still apply.
        if (refreshing) { generation++; refreshing = false; log('poll_superseded_by_action',{op}); }
        const text = $('prompt').value;
        if (['submit','steer'].includes(op) && ((!text.trim() && !composition?.active?.document?.parts.some(p=>p.type==='image')) || new TextEncoder().encode(text).length > 65536)) return notice('Message must contain 1–65536 UTF-8 bytes.');
        if (decision && (decision.expires_at_ms <= Date.now() || decision.incarnation !== incarnation || decision.run_id !== snapshot.run?.run_id)) return notice('Decision expired. Refresh before responding.');
        busy = true; controls();
        const fields = ['submit','steer'].includes(op) ? {prompt:text} : decision ? {decision_id:decision.decision_id,response:answer,expires_at_ms:Math.min(Date.now()+60000,decision.expires_at_ms)} : extra;
        let command, sent;
        const active = client, activeJournal = journal, id = selected, vessel = selectedVessel, mine = generation, originSnapshot = structuredClone(snapshot), originIncarnation = incarnation;
        try {
            if (['submit','steer'].includes(op)) {
                sent = await composition.beforeSend();
                if (sent.record.document.parts.some(p=>p.type==='image')) {
                    const content = await composition.promote(sent,id); op = 'submit_content'; delete fields.prompt; fields.content = content;
                }
            }
            if (mine !== generation || active !== client || id !== selected || vessel !== selectedVessel) throw new Error('Selection changed while preparing the action; nothing sent.');
            if (sent && op !== 'submit_content') fields.prompt = sent.record.document.parts.filter(p=>p.type==='text').map(p=>p.text).join('');
            command = mutation(op,originSnapshot,originIncarnation,fields); activeJournal.prepare(command.command);
            if(sent) admittedDrafts.set(command.command.command_id,sent);
            const response = await active.exchange(command);

            if (response.error != null) notice(`Vessel refused the action: ${clean(response.error)}. Draft retained; review model support and attachments.`);
            else if (response.outcome_unknown || !resolved(response,command.command.command_id,id)) notice('Outcome uncertain. Checking receipts only; the action will not be resent.');
            else { notice(`Receipt ${command.command.command_id}: ${receiptStatus(response) || 'acknowledged'}. ${receiptStatus(response) === 'not_applied' ? 'Message was not applied; draft retained.' : 'Execution may still be pending.'}`); if (sent && ['accepted','queued','applied'].includes(receiptStatus(response))) { await composition.admitted(sent); forgetSent(command.command.command_id); } }
            if (resolved(response,command.command.command_id,id)) activeJournal.settle(command.command.command_id);
            return response.error == null && resolved(response,command.command.command_id,id);
        } catch (error) { notice(`${error.message || 'Action not confirmed.'} Draft retained. Any recorded command remains pending; reconnect checks receipts without resending.`); }
        finally { busy = false; stale = true; controls(); refresh(); }
    }
    $('composer').addEventListener('submit', event => { event.preventDefault(); send(); });
    // Flux handles Enter; suppress its handler while an IME is committing text.
    $('prompt').addEventListener('keydown', event => {
        if (event.key === 'Enter' && (event.isComposing || event.keyCode === 229)) event.stopImmediatePropagation();
    }, {capture:true});
    $('access-mode').addEventListener('change', () => {
        const access = $('access-mode').value;
        if (!selected && composition?.active?.document.target.type === 'new_chat') { composition.active.access = access; controls(); return; }
        if (['read-only','approval','unrestricted'].includes(access) && access !== snapshot?.access) act('set_access',null,null,{access});
        controls(); // Only a refreshed owner snapshot confirms the new mode.
    });
    // Popover form fields must not submit the enclosing message composer.
    for (const popover of $('composer').querySelectorAll('[data-flux-popover]')) {
        popover.addEventListener('keydown', event => {
            if (event.key === 'Enter' && !event.target.closest('button')) event.preventDefault();
        });
    }
    let reasoningTarget;
    $('change-reasoning').addEventListener('click', () => {
        reasoningTarget = {vessel:selectedVessel, session_id:selected, incarnation, revision:snapshot?.revision};
        const config = settings.configuration();
        reasoningTarget.draftKey = captureDraft()?.key;
        const inference = config ? {...config.settings,reasoning_efforts:config.reasoning_efforts} : snapshot?.inference;
        const levels = [...new Set(['', ...(inference?.reasoning_efforts || []), ...(inference?.reasoning_effort ? [inference.reasoning_effort] : [])])];
        setReasoning($('quick-reasoning'), levels.map(value => ({value,label:value || 'Provider default'})), inference?.reasoning_effort || '');
        $('reasoning-status').textContent = 'Applies to the next run.';
        $('reasoning-save').disabled = false;
    });
    $('reasoning-save').addEventListener('click', async () => {
        const t = reasoningTarget;
        if (!selected && t?.draftKey === captureDraft()?.key && settings.setReasoning(reasoningValue($('quick-reasoning')))) { $('reasoning-popover').hidePopover?.(); return; }
        if (!t || t.vessel !== selectedVessel || t.session_id !== selected || t.incarnation !== incarnation || t.revision !== snapshot?.revision || !actionable() || running()) {
            $('reasoning-status').textContent = 'Voyage changed. Reopen this popover to reload choices.'; return;
        }
        $('reasoning-save').disabled = true;
        const inference = snapshot.inference;
        const ok = await act('set_account_inference',null,null,{account:inference.account,model:inference.model,reasoning_effort:reasoningValue($('quick-reasoning')) || null,service_tier:inference.service_tier || null});
        $('reasoning-status').textContent = ok ? 'Settings confirmed.' : 'Not confirmed. Review the voyage status before trying again.';
        if (ok) $('reasoning-popover').hidePopover?.();
    });
    $('cancel').addEventListener('click',() => act('cancel'));
    $('earlier').addEventListener('click',earlier); $('more-output').addEventListener('click',moreOutput);
    $('voyage-search').addEventListener('input',renderVoyages);
    $('reconnect').addEventListener('click',() => fleet.reconnect());
    window.addEventListener('storage', () => controls());
    const timer = setInterval(() => { controls(); if (!document.hidden) { fleet.poll(); if (pending().length || Date.now() - lastFresh >= 30000 || stale) refresh(); } },1000);
    window.addEventListener('pagehide',() => { scrolling.dispose(); composition?.dispose(); stopStream(); generation++;clearInterval(timer);fleet.close(); });
    document.addEventListener('visibilitychange',() => { if (!document.hidden) { stale=true;controls();fleet.poll();refresh(); } });
    composition = composer(root, {fleet,select,notice,changed:()=>controls(),current:()=>({vessel:selectedVessel,session_id:selected,incarnation,run_id:snapshot?.run?.run_id,running:running()})});
    settings = voyageSettings(root,fleet,{
        current:() => ({vessel:selectedVessel,session_id:selected,incarnation,revision:snapshot?.revision,inference:snapshot?.inference}),
        select,
        draft: (vessel,workspace)=>composition.newChat(vessel,workspace),
        unavailable: message => notice(message),
        resumeDraft: () => { const draft = composition.newDraft; return draft ? {vessel:draft.vessel,workspace:draft.document.target.workspace} : null; },
        created: (...args)=>composition.created(...args),
        captureDraft,
        prepared: ()=>{
            notice(''); controls();
            // Flux propagates disabled state through a MutationObserver. Focus
            // its actual input after that propagation, not the custom-element host.
            queueMicrotask(() => {
                if (!selected) {
                    const prompt = $('prompt');
                    if (typeof prompt.focusInput === 'function') prompt.focusInput();
                    else (prompt.querySelector('textarea') || prompt).focus();
                }
            });
        },
        apply:async (target,fields) => {
            if (target.vessel !== selectedVessel || target.session_id !== selected || target.incarnation !== incarnation || target.revision !== snapshot?.revision || !actionable() || running()) return false;
            return act('set_account_inference',null,null,fields);
        },
    });
    connectionsChanged(); fleet.start();
}
const root = typeof document !== 'undefined' && document.querySelector('#helm-client');
if (root) mount(root);
