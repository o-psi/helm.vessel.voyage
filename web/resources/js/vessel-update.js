import {request, uuid} from './vessel-client.js';

// A stored id observes one server-owned operation. Reload/reconnect never repeats
// approval or starts another update, even when the original reply was lost.
export function vesselUpdate(root, {show, resume}) {
    const $ = id => root.querySelector(`#${id}`);
    let context = null, generation = 0, pending = null, timer = null, busy = false;
    const storageKey = c => `helm-web:update:${root.dataset.tenantId}:${c.id}:${c.vessel_id}`;
    const text = (id, value) => { $(id).textContent = String(value ?? ''); };
    const active = (c, n) => context?.c === c && generation === n;
    function remember(record) {
        pending = record;
        if (record?.operation_id) localStorage.setItem(storageKey(context.c), JSON.stringify({operation_id:record.operation_id}));
    }
    function render(record) {
        const phase = record?.phase || 'idle';
        text('update-status', record?.message || 'Check for a verified update. Nothing is installed until you approve the prepared build.');
        $('update-source').hidden = !['idle','discarded','complete','failed'].includes(phase);
        $('update-review').hidden = phase !== 'ready';
        $('update-refresh').hidden = ['idle','discarded'].includes(phase);
        $('update-continue').hidden = phase !== 'complete';
        text('update-version',record?.version);
        text('update-description',record?.description);
        text('update-services',record?.services?.length ? `Services to restart: ${record.services.join(', ')}` : '');
        $('update-approve').disabled = busy;
        $('update-discard').disabled = busy;
        $('update-check').disabled = busy;
    }
    async function exchange(c, op, fields) {
        if (!c.client) throw Error('Waiting for this Vessel to reconnect. Your update request is retained.');
        const reply = await c.client.exchange(request(op, fields));
        if (reply.protocol !== 1 || reply.error != null || reply.outcome_unknown !== false) throw Error('The Vessel could not confirm the request. Check this update’s status; it has not been repeated.');
        return reply.result;
    }
    async function refresh() {
        const {c,caps} = context || {};
        if (!c || !caps.remote_updates || busy) return;
        const n=generation;
        try {
            const record = await exchange(c,'update_status',{operation_id:pending?.operation_id || '00000000-0000-0000-0000-000000000000'});
            if (!active(c,n)) return;
            if (pending?.operation_id && record.operation_id !== pending.operation_id) throw Error('Update identity changed. Reopen update review.');
            if (record.phase === 'complete') {
                const capabilities = await exchange(c,'capabilities',{});
                if (!active(c,n)) return;
                if (capabilities.vessel_id !== c.vessel_id || capabilities.running_release !== record.release_id || !capabilities.features?.includes('execution_profiles')) {
                    throw Error('The approved update is recorded, but this connection has not verified the new Vessel yet. Check again after it reconnects.');
                }
            }
            remember(record); render(record);
        } catch (error) { if (active(c,n)) text('update-status',error.message); }
        if (active(c,n) && pending?.operation_id && !['ready','complete','failed','discarded','unconfirmed'].includes(pending.phase)) {
            clearTimeout(timer); timer=setTimeout(refresh,3000);
        }
    }
    async function mutate(op, fields) {
        if (busy || !context?.caps.remote_updates) return;
        const {c}=context, n=generation;
        busy=true; render(pending);
        try {
            const record=await exchange(c,op,fields);
            if (active(c,n)) { remember(record.phase === 'complete' ? {...record,phase:'applying'} : record); render(pending); }
        } catch(error) { if (active(c,n)) text('update-status',error.message); }
        finally { if (active(c,n)) { busy=false; $('update-approve').disabled=false; $('update-discard').disabled=false; $('update-check').disabled=false; } }
        // Resolve the already journalled identity after an uncertain response.
        if (active(c,n)) await refresh();
    }
    $('update-check').addEventListener('click',() => {
        if (busy) return;
        const record={operation_id:uuid(),phase:'preparing'};
        try { remember(record); } catch { return text('update-status','Browser recovery storage is unavailable. Enable it before preparing an update.'); }
        mutate('update_prepare',{operation_id:record.operation_id,channel:$('update-channel').value});
    });
    $('update-approve').addEventListener('click',() => {
        if (pending?.phase !== 'ready' || busy) return;
        const fields={operation_id:pending.operation_id,release_id:pending.release_id};
        pending={...pending,phase:'applying'}; render(pending);
        mutate('update_apply',fields);
    });
    $('update-discard').addEventListener('click',() => pending?.phase === 'ready' && mutate('update_discard',{operation_id:pending.operation_id}));
    $('update-refresh').addEventListener('click',refresh);
    $('update-continue').addEventListener('click',() => resume());
    $('setup-update-open').addEventListener('click',() => { show(); refresh(); });
    return {
        bind(c,caps) {
            ++generation; clearTimeout(timer); busy=false; pending=null; context=c && caps ? {c,caps}:null;
            $('setup-update-open').hidden = !context;
            if (!context) return;
            text('update-vessel-name',`Update ${c.name}`); text('update-current',`Installed Vessel version: ${caps.version || 'unknown'}`);
            try { pending=JSON.parse(localStorage.getItem(storageKey(c)) || 'null'); } catch { pending=null; }
            render(null);
            if (!caps.remote_updates) {
                $('update-source').hidden=true; $('update-refresh').hidden=true;
                text('update-status',caps.scope !== 'owner' ? 'Only this Vessel’s account owner can approve an update.' : 'This version predates remote updates. Its updater needs a one-time remote administrator installation; a browser reconnect cannot add that capability. Your draft is retained.');
            }
        },
        required() { show(); },
    };
}
