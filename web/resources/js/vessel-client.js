import {connectionDiagnostic as log} from './connection-diagnostics.js';
// Socket correlation is ephemeral. Durable command IDs are separate and never replayed.
export const uuid = () => crypto.randomUUID();
export const request = (op, fields = {}) => ({protocol: 1, command: {op, ...fields}});
export function voyageResult(response, session, incarnation = null) {
    if (response.protocol !== 1 || response.error != null || response.outcome_unknown !== false) throw new Error('Vessel refused or could not confirm this read.');
    const envelope = response.result;
    if (envelope?.session_id !== session || typeof envelope.incarnation !== 'string' || (incarnation && incarnation !== envelope.incarnation)) throw new Error('Voyage identity changed. Refresh before acting.');
    return envelope;
}
export function mutation(op, snapshot, incarnation, fields = {}) {
    if (!Number.isSafeInteger(snapshot.revision) || !snapshot.session_id) throw new Error('A current snapshot is required.');
    const command = {session_id: snapshot.session_id, command_id: uuid(), expected_revision: snapshot.revision, expires_at_ms: Date.now() + 60000};
    if (!['submit','submit_content'].includes(op)) command.incarnation = incarnation;
    if (['steer','cancel','respond'].includes(op)) command.run_id = snapshot.run?.run_id;
    return request(op, {...command, ...fields});
}
export function resolved(response, commandId, sessionId, receipt = false) {
    if (response?.protocol !== 1 || response.outcome_unknown !== false) return false;
    if (response.error != null) return !receipt;
    const envelope = response.result, value = envelope?.result;
    const matches = envelope?.session_id === sessionId && (value?.command_id === commandId || value?.request?.receipt_id === commandId || value?.record?.request?.receipt_id === commandId);
    return Boolean(matches && (!receipt || ['accepted', 'requested', 'already_terminal', 'applied', 'deleted', 'transferred', 'queued', 'not_applied', 'unknown_after_restart'].includes(value?.status)));
}
export class IntentJournal {
    constructor(storage, vesselId) { this.storage = storage; this.key = `helm-web:intent:${vesselId}:`; }
    entries() {
        const entries = [];
        for (let i = 0; i < this.storage.length; i++) {
            const key = this.storage.key(i);
            if (!key?.startsWith(this.key)) continue;
            const value = JSON.parse(this.storage.getItem(key));
            if (!value || typeof value.command_id !== 'string' || typeof value.session_id !== 'string' || key !== this.key + value.command_id) throw new Error('Command journal is invalid. Do not resend uncertain work.');
            entries.push(value);
        }
        return entries;
    }
    prepare(command) {
        if (this.entries().length >= 128) throw new Error('Command journal full. Reconcile outstanding receipts first.');
        // Each intent has its own key: other tabs cannot overwrite a shared array.
        // Prompts, answers and conversation text never enter persistent storage.
        this.storage.setItem(this.key + command.command_id, JSON.stringify({session_id:command.session_id, command_id:command.command_id, op:command.op, created_at:Date.now()}));
    }
    settle(commandId) { this.storage.removeItem(this.key + commandId); }
}
export class VesselSocket {
    constructor(socket, onDisconnect) {
        this.socket = socket; this.pending = new Map(); this.onDisconnect = onDisconnect;
        socket.addEventListener('message', event => {
            try {
                const frame = JSON.parse(event.data);
                if (frame.type !== 'reply') return;
                const pending = this.pending.get(frame.request_id);
                if (pending) { log('reply',{request_id:frame.request_id,op:pending.op,elapsed_ms:Date.now()-pending.started,unknown:frame.response?.outcome_unknown === true,error:frame.response?.error != null}); clearTimeout(pending.timer); this.pending.delete(frame.request_id); pending.resolve(frame.response); }
            } catch { socket.close(); }
        });
        socket.addEventListener('close', event => {
            for (const pending of this.pending.values()) { clearTimeout(pending.timer); pending.reject(new Error('Connection lost; command outcome may be unknown.')); }
            this.pending.clear();
            const reasons = ['lease expired','heartbeat timeout','upstream closed','upstream unavailable','gateway refused','gateway capacity','rate exceeded','invalid upstream frame'];
            const reason = reasons.includes(event.reason) ? event.reason : 'connection closed';
            log('socket_closed',{code:event.code,reason}); this.onDisconnect(reason);
        });
    }
    exchange(value) {
        return new Promise((resolve, reject) => {
            if (this.socket.readyState !== 1 || this.pending.size >= 16) return reject(new Error('Connection unavailable.'));
            const id = uuid();
            const timer = setTimeout(() => { log('reply_timeout',{request_id:id,op:value.command.op,pending:this.pending.size}); this.pending.delete(id); reject(new Error('Reply timed out; command outcome may be unknown.')); }, 30000);
            this.pending.set(id, {resolve, reject, timer,op:value.command.op,started:Date.now()});
            log('request',{request_id:id,op:value.command.op,pending:this.pending.size});
            try { this.socket.send(JSON.stringify({type: 'command', request_id: id, request: value})); }
            catch (error) { clearTimeout(timer); this.pending.delete(id); reject(error); }
        });
    }
    close() { this.socket.close(); }
}
