import {request, voyageResult, mutation, uuid} from './vessel-client.js';

const activeRun = s => ['starting','running','cancelling'].includes(s?.run?.state);
const archived = v => Boolean(v.process.archive || v.snapshot?.lifecycle?.archived);
const terminal = new Set(['applied','accepted','requested','already_terminal','deleted','not_applied']);
const descriptions = {
    access:'Change execution policy on the owning Vessel. Configured roots and server limits still apply.',
    rename:'Rename this voyage.', archive:'Archive an idle voyage, or restore an archived voyage.',
    branch:'Create an independent voyage with copied canonical history and current settings. No active run or pending approvals are copied. Provider continuation is reset; future turns can incur provider charges.',
    cancel:'Request cancellation of this exact run. Requested cancellation is not proof of completed cleanup.',
    clear:'Remove current messages and provider continuation, preserving voyage identity and prior run/receipt evidence. This is not forensic erasure. Branch first if you need a copy.',
    compact:'Compact working context while preserving canonical conversation history. This can reset provider continuation.',
    delete:'Permanently delete this voyage’s conversation history. This cannot be undone.', details:'Fresh public voyage details.',
};
export function actionReason(action, view, connection) {
    if (!connection.client) return 'Vessel is offline.';
    if (!view) return 'Loading fresh voyage state…';
    if (action === 'details' && view.process) return null;
    if (view.error) return view.error;
    if (connection.journal.entries().some(e => e.session_id === view.process.session_id)) return 'An earlier command is unresolved. Check its receipt; do not repeat it.';
    const caps = view.caps;
    if (['access','branch'].includes(action) && caps.scope !== 'owner') return 'Executing-account owner authority is required.';
    const rights = {rename:['lifecycle'],archive:['lifecycle'],branch:['create','history','lifecycle'],cancel:['cancel'],clear:['lifecycle'],compact:['lifecycle'],delete:['lifecycle'],access:['execute']};
    if ((caps.scope !== 'owner' || Array.isArray(caps.rights)) && rights[action]?.some(r => !caps.rights?.includes(r))) return 'This connection lacks the required authority.';
    if (action === 'archive' && view.process.state === 'stopped' && view.process.archive) return null;
    if (archived(view) && action !== 'archive') return 'Restore this archived voyage first.';
    if (!['live','suspended'].includes(view.process.state)) return 'Voyage is unavailable; unavailable is not stopped.';
    const s = view.snapshot;
    if (!s) return 'No current snapshot.';
    if (s.lifecycle?.deleted) return 'History has been deleted.';
    if (action === 'cancel') return activeRun(s) ? null : 'There is no active run to cancel.';
    if (activeRun(s) && action !== 'access') return 'Wait for the current run to finish.';
    if (s.pending_cleanup_run && !(action === 'access' && activeRun(s))) return 'Waiting for confirmed cleanup.';
    return null;
}
async function read(client, op, fields = {}) {
    const response = await client.exchange(request(op, fields));
    if (response?.protocol !== 1 || response.outcome_unknown !== false || response.error) throw new Error(response?.error?.message || response?.error || 'Vessel could not confirm this read.');
    return response.result;
}
async function inspect(connection, id) {
    const client = connection.client;
    if (!client) throw new Error('Vessel is offline.');
    const caps = await read(client,'capabilities');
    const process = await read(client,'inspect',{session_id:id});
    if (process.session_id !== id || !process.incarnation) throw new Error('Voyage identity changed.');
    let snapshot = null, error = null;
    try { if (['live','suspended'].includes(process.state)) {
        snapshot = voyageResult(await client.exchange(request('snapshot',{session_id:id,incarnation:process.incarnation})),id,process.incarnation).result;
        if (snapshot.session_id !== id || !Number.isSafeInteger(snapshot.revision)) throw new Error('Invalid snapshot.');
    }
    } catch (failure) { error = failure.message; }
    if (client !== connection.client) throw new Error('Connection changed; reopen the action.');
    return {caps,process,snapshot,client,error};
}
export function sidebarActions(root, {changed = () => {}, modal = name => window.Flux.modal(name)} = {}) {
    const $ = id => root.querySelector(`#sidebar-${id}`);
    let epoch = 0, current = null, sending = false;
    const status = text => { $('action-status').textContent = text; };
    function invalidate() { epoch++; current = null; $('submit').disabled = true; }
    function bind(node, connection, item) {
        const row = node.querySelector('[data-voyage-row]'), menu = node.querySelector('[data-flux-menu]');
        let loaded = null, ticket = 0;
        async function prepare() {
            const mine = ++ticket; loaded = null; render();
            try { const next = await inspect(connection,item.session_id); if (mine === ticket) loaded = next; }
            catch(error) { if (mine === ticket) loaded = {error:error.message}; }
            if (mine === ticket) render();
        }
        function render() {
            for (const control of menu.querySelectorAll('[data-voyage-action]')) {
                const reason = actionReason(control.dataset.voyageAction,loaded,connection);
                control.disabled = Boolean(reason); control.setAttribute('aria-disabled',String(Boolean(reason)));
                control.title = reason || descriptions[control.dataset.voyageAction];
                let hint = control.querySelector('[data-disabled-reason]');
                if (!hint) { hint=document.createElement('span'); hint.dataset.disabledReason=''; hint.className='block text-xs opacity-70'; control.append(hint); }
                hint.textContent = reason || '';
            }
        }
        row.addEventListener('contextmenu',prepare,true);
        function openMenu(event) {
            event.preventDefault(); event.stopPropagation(); const rect=row.getBoundingClientRect();
            row.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:rect.right,clientY:rect.bottom}));
            queueMicrotask(() => menu.focus());
        }
        node.querySelector('[data-voyage-actions]').addEventListener('click',openMenu);
        row.addEventListener('keydown',event => { if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) openMenu(event); });
        for (const control of menu.querySelectorAll('[data-voyage-action]')) control.addEventListener('click',() => {
            if (!control.disabled) open(connection,item,control.dataset.voyageAction);
        });
        render();
    }
    async function open(connection,item,action) {
        const mine = ++epoch; current = null; $('submit').disabled=true;
        $('action-title').textContent=({access:'Access modes',archive:'Archive / Restore',cancel:'Cancel run'})[action] || action[0].toUpperCase()+action.slice(1);
        $('action-target').textContent=`${item.name || item.session_id} · ${connection.name} · ${item.session_id}`;
        for (const name of ['name','access','branch','retain','confirm']) $(`${name}-field`).hidden=true;
        $('action-details').hidden=true; $('confirm').value=''; $('reconcile').hidden=false;
        status('Loading fresh voyage state…'); modal('sidebar-action').show();
        try {
            const view = await inspect(connection,item.session_id);
            if (mine !== epoch) return;
            current={connection,item,action,view,mine};
            const reason=actionReason(action,view,connection);
            status(reason || descriptions[action]);
            $('submit').hidden=action==='details';
            $('submit').textContent=action==='archive' ? (archived(view) ? 'Restore' : 'Archive') : 'Confirm';
            if (action==='details') { $('action-details').hidden=false; $('action-details').textContent=JSON.stringify({process:view.process,snapshot:view.snapshot,error:view.error},null,2); }
            if (['rename','branch'].includes(action)) { $('name-field').hidden=false; $('name').value=action==='rename' ? view.snapshot?.name || item.name || '' : ''; }
            if (action==='access') { $('access-field').hidden=false; $('access').value=view.snapshot?.access || 'approval'; }
            if (action==='compact') $('retain-field').hidden=false;
            if (action==='branch' && !reason) {
                $('branch-field').hidden=false; $('branch').replaceChildren(new Option('Full conversation',''));
                current.branchOffset=0; current.branchIndices=new Set();
                await loadBranchPoints();
            }
            if (['clear','delete','compact'].includes(action)) { $('confirm-field').hidden=false; $('confirm-label').textContent=`Type ${action==='compact' ? 'COMPACT' : action.toUpperCase()} to confirm`; }
            $('submit').disabled=Boolean(reason);
        } catch(error) { if(mine===epoch) {status(error.message); $('submit').disabled=true;} }
    }
    async function loadBranchPoints() {
        const target=current;
        if(!target || target.action!=='branch' || target.branchLoading) return;
        const {view,item,mine}=target, offset=target.branchOffset;
        target.branchLoading=true; $('branch-more').disabled=true;
        try {
            const page=voyageResult(await view.client.exchange(request('history',{session_id:item.session_id,incarnation:view.process.incarnation,expected_revision:view.snapshot.revision,offset,limit:128})),item.session_id,view.process.incarnation).result;
            if(mine!==epoch || current!==target) return;
            if(page.revision!==view.snapshot.revision || page.message_offset!==offset) throw new Error('History changed during branch review.');
            for(const [n,message] of (page.messages || []).entries()) if(message.role==='user') {
                const index=message.message_index ?? offset+n;
                if(!Number.isSafeInteger(index)||index<offset) throw new Error('Invalid canonical index.');
                target.branchIndices.add(index); $('branch').append(new Option(`Through user message ${index+1}`,String(index)));
            }
            $('branch-more').hidden=!page.has_more;
            if(page.has_more && (!Number.isSafeInteger(page.next_offset)||page.next_offset<=offset)) throw new Error('History paging did not advance.');
            target.branchOffset=page.next_offset;
        } finally {target.branchLoading=false; if(mine===epoch) $('branch-more').disabled=false;}
    }
    async function send(connection, command) {
        connection.journal.prepare({...command,sidebar_action:true});
        const response=await connection.client.exchange(request(command.op,Object.fromEntries(Object.entries(command).filter(([k])=>k!=='op'))));
        if (response?.protocol!==1 || response.outcome_unknown!==false) throw new Error('Outcome unknown. Check the receipt; do not repeat this action.');
        if (response.error) { if(!['branch','restart'].includes(command.op)) connection.journal.settle(command.command_id); throw new Error(response.error.message || String(response.error)); }
        return response;
    }
    async function execute(event) {
        event.preventDefault(); if (!current || sending) return;
        const target=current, {connection,item,action,view,mine}=target;
        sending=true; $('submit').disabled=true;
        try {
            if (['clear','delete','compact'].includes(action) && $('confirm').value !== (action==='compact'?'COMPACT':action.toUpperCase())) throw new Error('Enter the exact confirmation word.');
            const fields={};
            if (['rename','branch'].includes(action)) { fields.name=$('name').value.trim() || null; if(fields.name && new TextEncoder().encode(fields.name).length>256) throw new Error('Name exceeds 256 UTF-8 bytes.'); if(action==='rename' && !fields.name) throw new Error('Enter a name.'); }
            if (action==='compact') { fields.retain=Number($('retain').value); fields.preserve_canonical=true; if(!Number.isInteger(fields.retain)||fields.retain<0||fields.retain>4294967295||!$('retain').value) throw new Error('Enter a valid message count.'); }
            if (action==='branch') { fields.branch_id=uuid(); fields.through_message=$('branch').value===''?null:Number($('branch').value); if(fields.through_message!==null && (!Number.isSafeInteger(fields.through_message)||fields.through_message<0||!target.branchIndices?.has(fields.through_message))) throw new Error('Invalid branch boundary.'); }
            if (action==='access') fields.access=$('access').value;
            if (['clear','delete'].includes(action)) fields.confirm_session_id=item.session_id;
            const fresh=await inspect(connection,item.session_id);
            if(mine!==epoch || current!==target || fresh.client!==view.client) throw new Error('Selection or connection changed; nothing sent. Reopen the action.');
            if(fresh.process.incarnation!==view.process.incarnation || fresh.snapshot?.revision!==view.snapshot?.revision || fresh.process.state!==view.process.state) throw new Error('Voyage changed since review. Reopen this action.');
            const reason=actionReason(action,fresh,connection); if(reason) throw new Error(reason);
            let base=fresh, commandId=uuid();
            if(action==='archive' && fresh.process.state==='stopped' && fresh.process.archive) {
                const reply=await send(connection,{op:'restart',command_id:commandId,session_id:item.session_id,incarnation:fresh.process.incarnation});
                if(reply.result?.session_id!==item.session_id || !reply.result.incarnation || reply.result.incarnation===fresh.process.incarnation) throw new Error('Restart outcome unconfirmed. Check pending receipt.');
                base=await inspect(connection,item.session_id);
                if(base.process.incarnation!==reply.result.incarnation || !base.snapshot) throw new Error('Restored process identity unconfirmed.');
                connection.journal.settle(commandId);
                if(mine!==epoch || current!==target) throw new Error('Restart observed, but selection changed. Reopen Restore to finish.');
            }
            if(action==='archive') fields.archived=!archived(fresh);
            const op=action==='access'?'set_access':action;
            const command=mutation(op,base.snapshot,base.process.incarnation,{...fields,command_id:commandId}).command;
            const response=await send(connection,command);
            if(action==='branch') {
                if(response.result?.session_id!==fields.branch_id || !response.result.incarnation) throw new Error('Branch start unconfirmed. Check pending receipt.');
                connection.journal.settle(commandId);
            } else {
                const envelope=voyageResult(response,item.session_id,base.process.incarnation);
                if(envelope.result?.command_id!==commandId || !terminal.has(envelope.result.status)) throw new Error('Outcome unconfirmed. Check pending receipt.');
                connection.journal.settle(commandId);
                if(['not_applied','unknown_after_restart'].includes(envelope.result.status)) throw new Error(`Command ${envelope.result.status}; inspect before any new action.`);
            }
            if(mine===epoch) {status(action==='branch'?`Branch created: ${fields.branch_id}`:action==='cancel'?'Cancellation requested; cleanup may still be pending.':'Action confirmed by Vessel.'); current=null;}
            changed();
        } catch(error) { if(mine===epoch) status(error.message); }
        finally { sending=false; if(mine===epoch && current) $('submit').disabled=Boolean(actionReason(action,view,connection)); }
    }
    async function reconcile() {
        if(!current || sending) return;
        const {connection,item,mine}=current; sending=true; $('submit').disabled=true;
        try {
            const entries=connection.journal.entries().filter(e=>e.session_id===item.session_id && e.sidebar_action);
            if(!entries.length) {status('No pending sidebar command. Reopen the action for a fresh review.');return;}
            for(const entry of entries) {
                if(entry.op==='restart') {
                    const process=await read(connection.client,'inspect',{session_id:item.session_id});
                    if(process.incarnation!==entry.incarnation && ['live','suspended'].includes(process.state)) {connection.journal.settle(entry.command_id);if(mine===epoch) status('A restarted process is observed. Reopen Restore to finish; no command was replayed.');}
                    else if(mine===epoch) status('Restart remains uncertain. No automatic replay.');
                    continue;
                }
                const reply=await read(connection.client,'receipt',{session_id:item.session_id,command_id:entry.command_id});
                if(reply.session_id!==item.session_id || reply.result?.command_id!==entry.command_id) throw new Error('Receipt identity mismatch.');
                const receipt=reply.result;
                if(entry.op==='branch' && receipt.status==='snapshot_committed') {
                    if(receipt.branch_id!==entry.branch_id) throw new Error('Branch receipt identity mismatch.');
                    const child=await read(connection.client,'inspect',{session_id:entry.branch_id});
                    if(child.session_id!==entry.branch_id || !child.incarnation) throw new Error('Source frozen; branch start is still uncertain.');
                    connection.journal.settle(entry.command_id); if(mine===epoch) status(`Branch observed: ${entry.branch_id}. No command was replayed.`);
                } else if(entry.op==='branch' && !['not_applied','unknown_after_restart'].includes(receipt.status)) { if(mine===epoch) status('Branch outcome remains uncertain; source receipt is not proof of child creation.'); } else if(terminal.has(receipt.status)) {connection.journal.settle(entry.command_id);if(mine===epoch) status(`Receipt: ${receipt.status}. Reopen for fresh review. This is not a claim of completed execution.`);}
                else if(mine===epoch) status(`Receipt: ${receipt.status || 'unknown'}. Still pending; no replay.`);
            }
            changed();
        } catch(error) { if(mine===epoch) status(`Outcome still uncertain: ${error.message}`); }
        finally {sending=false;}
    }
    $('branch-more').addEventListener('click',()=>loadBranchPoints().catch(error=>{status(error.message); $('submit').disabled=true;}));
    $('action-form').addEventListener('submit',execute);
    $('reconcile').addEventListener('click',reconcile);
    $('dismiss').addEventListener('click',()=>{invalidate();modal('sidebar-action').close();});
    return {bind,open,invalidate,reconcile};
}
