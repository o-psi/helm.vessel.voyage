import {request, uuid} from './vessel-client.js';

const terminal = new Set(['succeeded','cancelled','expired','denied','uncertain']);
const verification = 'https://auth.openai.com/codex/device';
// Only public enrollment envelopes are durable. Private responses live in this view alone.
export function accountEnrollment(root, {context, refreshed}) {
    const $ = id => root.querySelector(`#enrollment-${id}`);
    let generation = 0, active = null, busy = false, timer, stage = 'loading';
    const key = c => `helm-web:enrollment:${root.dataset.tenantId}:${c.connection.id}:${c.connection.vessel_id}:${c.workspace}`;
    function clear() {
        ++generation; clearTimeout(timer);
        $('code').textContent = ''; $('link').removeAttribute('href'); $('private').hidden = true;
    }
    function render(next = stage) {
        stage = next;
        const setup = stage === 'setup', pending = ['pending','recovery'].includes(stage);
        $('setup').hidden = !setup;
        $('start').hidden = !setup;
        $('check').hidden = !pending;
        $('cancel').hidden = !pending;
        $('title').textContent = ({loading:'Connect ChatGPT',setup:'Connect ChatGPT',pending:'Finish signing in',recovery:'Resume sign-in',done:'Account connected',unavailable:'Sign-in unavailable'})[stage];
        $('check').textContent = stage === 'pending' ? 'I’ve signed in' : 'Check sign-in';
        for (const id of ['start','check','cancel','label','provider']) $(id).disabled = busy || stage === 'loading';
        $('panel').setAttribute('aria-busy',String(busy || stage === 'loading'));
    }
    function message(text) { $('status').textContent = text; }
    function current(a, n) { const c = context(); return active === a && generation === n && c?.connection.client === a.client && c.connection === a.context.connection && c.workspace === a.context.workspace; }
    async function exchange(a, op, fields) {
        const reply = await a.client.exchange(request(op, fields));
        if (reply.protocol !== 1 || reply.error != null || reply.outcome_unknown !== false) throw Error();
        return reply.result;
    }
    function saved(c) {
        const record = JSON.parse(localStorage.getItem(key(c)) || 'null');
        if (!record) return null;
        if (!record.command_id || !record.enrollment_id || !record.connection_id || record.workspace !== c.workspace || typeof record.alias !== 'string' || typeof record.label !== 'string' || !record.cancel_id) throw Error();
        // Whitelist fields even when browser storage has been modified.
        return Object.fromEntries(['command_id','enrollment_id','connection_id','workspace','alias','label','cancel_id'].map(k=>[k,record[k]]));
    }
    function envelope(record) { const {cancel_id, ...fields} = record; return fields; }
    async function show(a, n) {
        const result = await exchange(a,'private_account_enrollment',{enrollment_id:a.record.enrollment_id,workspace:a.record.workspace});
        if (!current(a,n)) { if (active === a) clear(); return; }
        const status = result?.status;
        if (status?.enrollment_id !== a.record.enrollment_id) throw Error();
        clear(); n = generation;
        if (terminal.has(status.state)) {
            render(status.state === 'uncertain' ? 'recovery' : status.state === 'succeeded' ? 'done' : 'setup');
            message(status.state === 'succeeded' ? 'ChatGPT account created. Reloading account choices…' : `Sign-in ${status.state}. Provider effects may already have occurred; no sign-in is retried automatically.`);
            if (status.state !== 'uncertain') localStorage.removeItem(key(a.context));
            if (status.state === 'succeeded') await refreshed(status.account_id);
            return;
        }
        if (!['starting','pending','exchanging'].includes(status.state)) throw Error();
        render('pending');
        message(status.state === 'pending' ? 'After signing in on ChatGPT, come back here and choose “I’ve signed in”.' : 'ChatGPT is processing your sign-in. Check again in a moment.');
        if (status.state === 'pending' && Number.isFinite(status.expires_at) && status.expires_at * 1000 > Date.now()) {
            if (result.verification_uri !== verification || typeof result.user_code !== 'string' || !/^[A-Za-z0-9-]{1,32}$/.test(result.user_code)) throw Error();
            $('code').textContent = result.user_code; $('link').href = verification; $('private').hidden = false;
            timer = setTimeout(()=>{clear();render('recovery');message('Code expired. Check sign-in for its final status.');},Math.min(status.expires_at*1000-Date.now(),2147483647));
        }
    }
    async function runLocked(kind) {
        if (busy) return;
        clear(); busy = true; render();
        message(kind === 'start' ? 'Starting secure sign-in…' : kind === 'cancel' ? 'Cancelling sign-in…' : 'Checking your sign-in…');
        const c = context();
        try {
            if (!c?.connection?.client || !c.workspace) throw Error();
            const a = active = {context:c,client:c.connection.client,record:null};
            const n = generation;
            a.client.socket?.addEventListener('close',()=>{if(active===a){clear();render('recovery');message('Connection lost. Reconnect, then Check sign-in; do not start again.');}},{once:true});
            a.record = saved(c);
            if (kind === 'start') {
                if (a.record) { render('recovery'); message('An earlier sign-in is retained. Check or cancel it before starting another.'); return; }
                const alias = `chatgpt-${uuid()}`, label = $('label').value.trim();
                if (!label || label.length > 128) {message('Give this account a name, such as Personal or Work.');$('label').focus();return;}
                const caps = await exchange(a,'capabilities',{});
                if (!current(a,n)) return;
                if (caps.vessel_id !== c.connection.vessel_id || (caps.scope !== 'owner' && !caps.rights?.includes('account_enroll'))) {message('This Vessel connection does not permit account enrollment.');return;}
                const catalogue = await exchange(a,'accounts',{workspace:c.workspace,transport:'chatgpt_oauth'});
                if (!current(a,n)) return;
                const provider = catalogue.connections?.find(p=>p.id===$('provider').value && p.transports?.includes('chatgpt_oauth'));
                if (!provider) {message('Choose an allowed ChatGPT provider connection.');return;}
                a.record = {command_id:uuid(),enrollment_id:uuid(),workspace:c.workspace,connection_id:provider.id,alias,label,cancel_id:uuid()};
                localStorage.setItem(key(c),JSON.stringify(a.record));
                await exchange(a,'enroll_account',envelope(a.record));
            } else {
                if (!a.record) {message('No retained sign-in for this Vessel and workspace.');return;}
                await exchange(a,'resolve_account_enrollment',envelope(a.record));
                if (kind === 'cancel') {
                    if (!current(a,n)) return;
                    await exchange(a,'cancel_account_enrollment',{command_id:a.record.cancel_id,enrollment_id:a.record.enrollment_id,workspace:c.workspace});
                }
            }
            if (current(a,n)) await show(a,n);
        } catch {
            clear(); render('recovery'); message('Sign-in could not be confirmed. Reconnect and use Check sign-in to resolve the original request; it will not be restarted.');
        } finally {busy=false;render();}
    }
    async function run(kind) {
        const c = context(), locks = root.ownerDocument.defaultView.navigator.locks;
        if (!c?.connection || !locks) { message('Account sign-in requires a secure browser with Web Locks support.'); return; }
        try { await locks.request(key(c), {ifAvailable:true}, lock => {
            if (!lock) { message('Another tab is checking this sign-in. Wait, then Check sign-in.'); return; }
            return runLocked(kind);
        }); } catch { clear(); message('Browser coordination failed. Check the original sign-in before continuing.'); }
    }
    for (const kind of ['start','check','cancel']) $(''+kind).addEventListener('click',()=>run(kind));
    function hide() { clear(); $('panel').hidden=true; $('panel').closest('[data-flux-popover]')?.hidePopover?.(); }
    $('close').addEventListener('click',hide);
    root.ownerDocument.addEventListener('visibilitychange',()=>{if(root.ownerDocument.hidden)clear();});
    return {
        hide,
        changed() {if(active && !current(active,generation))clear();},
        async open() {
            clear(); render('loading'); $('panel').hidden=false; $('provider').replaceChildren(); message('Loading ChatGPT connections…');
            const c=context(), n=generation;
            if(!c?.connection?.client || !c.workspace){render('unavailable');message('Choose a connected Vessel and workspace first.');return;}
            const a=active={context:c,client:c.connection.client};
            let phase = 'catalogue';
            try {
                const catalogue=await exchange(a,'accounts',{workspace:c.workspace,transport:'chatgpt_oauth'});
                if(!current(a,n))return;
                phase = 'picker';
                for(const p of catalogue.connections || []) if(p.transports?.includes('chatgpt_oauth')) {
                    const option=root.ownerDocument.createElement('option');option.value=p.id;option.textContent=p.label;$('provider').append(option);
                }
                $('provider-field').hidden = $('provider').options.length < 2;
                phase = 'storage';
                const retained = saved(c);
                phase = 'picker';
                render(retained ? 'recovery' : $('provider').options.length ? 'setup' : 'unavailable');
                if (!retained && !$('provider').options.length) { message('No ChatGPT connection is available on this Vessel. Ask its owner to enable ChatGPT sign-in.'); return; }
                if (!retained) $('label').focus();
                message(retained ? 'A prior sign-in is retained. Use Check sign-in; starting again is blocked.' : 'Next, you’ll sign in securely on ChatGPT. No password is entered here.');
            } catch {
                if (!current(a,n)) return;
                render('unavailable');
                message(phase === 'catalogue' ? 'The Vessel could not confirm the provider-account list. Check that this connection permits account enrollment for the selected workspace.' : phase === 'storage' ? 'The browser could not read the saved sign-in record. Do not clear site storage or start again: a previous sign-in may still be pending.' : 'The account list loaded, but the sign-in picker could not display it. Reload Helm to load the latest interface.');
            }
        },
    };
}
