import {uuid, request, voyageResult, mutation, resolved, receiptStatus} from '../js/vessel-client.js';
import {ConversationStream} from '../js/conversation-stream.js';

export type Connection = {id: string; name: string; client: any; journal: any; voyages: any[]; status: string};
export type Picture = {id: string; name: string; size: number; url: string; base64: string; uploadId: string; attachment?: any};
export type Tab = {key: string; vessel: string; session: string; title: string; draft: string; snapshot: any; incarnation: string | null; stale: boolean; busy: boolean; notice: string; freshAt: number; decisions: any[]; rights?: string[]; scope?: string; pictures: Picture[]};
// Transport state outlives React renders and selected tabs. No prompt is persisted.
export class Workspace {
    tabs = new Map<string, Tab>();
    private reads = new Map<string, Promise<void>>();
    private streams = new Map<string, {client: any; stop: () => void}>();
    private listeners = new Set<() => void>();
    private closed = false;
    private queued = new Set<string>();
    private epochs = new Map<string, number>();
    version = 0;
    constructor(private connections: () => Map<string, Connection>) {}
    subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
    getVersion = () => this.version;
    changed = () => { this.version++; this.listeners.forEach(listener => listener()); };
    open(vessel: string, session: string, title: string) {
        const key = JSON.stringify([vessel, session]);
        if (!this.tabs.has(key)) this.tabs.set(key, {key, vessel, session, title, draft: '', snapshot: null, incarnation: null, stale: true, busy: false, notice: '', freshAt: 0, decisions: [], pictures: []});
        this.changed(); void this.refresh(key); return key;
    }
    draft(key: string, value: string) { const tab = this.tabs.get(key); if (tab) { tab.draft = value; this.changed(); } }
    pending(tab: Tab) { return this.connections().get(tab.vessel)?.journal?.entries().filter((entry: any) => entry.session_id === tab.session) || []; }
    actionable(tab: Tab) {
        try { return Boolean(this.connections().get(tab.vessel)?.client && tab.snapshot && !tab.stale && !tab.busy && Date.now() - tab.freshAt < 35000 && !this.pending(tab).length); }
        catch { return false; }
    }
    permitted(tab: Tab, op: string) {
        const right = op==='respond'?'decide':op==='cancel'?'cancel':op==='set_account_inference'?'account_use':'execute';
        return tab.scope==='owner'||Boolean(tab.rights?.includes(right));
    }
    connectionChanged() {
        for (const tab of this.tabs.values()) {
            const stream = this.streams.get(tab.key), client = this.connections().get(tab.vessel)?.client;
            if (!client || stream?.client !== client) { tab.stale = true; stream?.stop(); this.streams.delete(tab.key); }
            if (client) void this.refresh(tab.key);
        }
        this.changed();
    }
    async refresh(key: string): Promise<void> {
        if (this.closed) return;
        if (this.reads.has(key)) { this.queued.add(key); return this.reads.get(key); }
        const tab = this.tabs.get(key), connection = tab && this.connections().get(tab.vessel), client = connection?.client;
        if (!tab || !client || tab.busy) return;
        const epoch = this.epochs.get(key) || 0;
        const task = (async () => {
            try {
                const envelope = voyageResult(await client.exchange(request('snapshot', {session_id: tab.session})), tab.session);
                const snapshot = envelope.result;
                const capabilities = await client.exchange(request('capabilities'));
                const caps = capabilities?.protocol===1 && capabilities.outcome_unknown===false && !capabilities.error ? capabilities.result : {scope:'unknown',rights:[]};
                if (snapshot.session_id !== tab.session || !Number.isSafeInteger(snapshot.revision)) throw new Error('Invalid snapshot identity.');
                let decisions: any[] = [];
                try { if(caps.scope==='owner'||caps.rights?.includes('decide')) decisions = voyageResult(await client.exchange(request('decisions', {session_id: tab.session})), tab.session, envelope.incarnation).result; } catch { /* History and decision authority are independent. */ }
                if (this.closed || this.connections().get(tab.vessel)?.client !== client || tab.busy || (this.epochs.get(key) || 0) !== epoch) return;
                if (tab.snapshot?.revision === snapshot.revision && tab.incarnation === envelope.incarnation) { snapshot.messages = tab.snapshot.messages; snapshot.message_offset = tab.snapshot.message_offset; }
                Object.assign(tab, {snapshot, scope:caps.scope, rights:caps.rights||[], incarnation: envelope.incarnation, title: snapshot.name || tab.title, decisions: Array.isArray(decisions) ? decisions : [], stale: false, freshAt: Date.now()});
                this.streams.get(key)?.stop(); this.streams.delete(key);
                const stream = new ConversationStream(); stream.seed(snapshot, envelope.incarnation);
                if (stream.valid && client.subscribe) {
                    const stop = client.subscribe(tab.session, envelope.incarnation, stream.cursor, (event: any) => {
                        if (this.closed || this.connections().get(tab.vessel)?.client !== client) return;
                        const action = stream.accept(event);
                        if (action === 'duplicate') return;
                        if (action === 'resync') { tab.stale = true; this.changed(); }
                        void this.refresh(key);
                    });
                    this.streams.set(key, {client, stop});
                }
            } catch (error) { tab.stale = true; tab.notice = error instanceof Error ? error.message : 'Read failed.'; }
            finally { this.changed(); }
        })();
        this.reads.set(key, task);
        try { await task; } finally { this.reads.delete(key); if(this.queued.delete(key) && !this.closed) queueMicrotask(()=>void this.refresh(key)); }
    }
    async act(key: string, op: string, fields: Record<string, unknown> = {}) {
        const tab = this.tabs.get(key);
        if (!tab || !this.actionable(tab) || !this.permitted(tab,op)) return;
        if (tab.snapshot.lifecycle?.archived || tab.snapshot.lifecycle?.deleted || (tab.snapshot.pending_cleanup_run && !['cancel','respond','set_access','steer'].includes(op) && !['running','starting','cancelling'].includes(tab.snapshot.run?.state))) { tab.notice='Voyage lifecycle or cleanup blocks this action. Review its status.'; this.changed(); return; }
        const connection = this.connections().get(tab.vessel)!;
        const draft = tab.draft;
        if (['submit', 'steer'].includes(op)) {
            if ((!draft.trim() && !tab.pictures.length) || new TextEncoder().encode(draft).length > 65536) { tab.notice = 'Message must contain 1–65536 UTF-8 bytes.'; this.changed(); return; }
            fields = {prompt: draft};
        }
        const pictures = [...tab.pictures];
        if (pictures.length && op === 'steer') { tab.notice = 'Pictures cannot be sent as steering. Wait for this run to finish.'; this.changed(); return; }
        const origin = structuredClone(tab.snapshot), incarnation = tab.incarnation;
        let command: any;
        this.epochs.set(key, (this.epochs.get(key) || 0) + 1);
        tab.busy = true; this.changed();
        try {
            if (op === 'submit' && pictures.length) {
                const content: any[] = draft ? [{type: 'text', text: draft}] : [];
                for (const picture of pictures) {
                    if (!picture.attachment) picture.attachment = await this.read(key, 'upload_image', {upload_id: picture.uploadId, name: picture.name, data_base64: picture.base64});
                    content.push({type: 'image', attachment: picture.attachment});
                }
                op = 'submit_content'; fields = {content};
            }
            if (tab.incarnation !== incarnation || tab.snapshot.revision !== origin.revision) throw new Error('Voyage changed while preparing attachments. Nothing submitted.');
            command = mutation(op, origin, incarnation, fields);
            connection.journal.prepare(command.command);
            const response = await connection.client.exchange(command);
            const known = resolved(response, command.command.command_id, tab.session);
            if (known && receiptStatus(response) !== 'unknown_after_restart') connection.journal.settle(command.command.command_id);
            const status = receiptStatus(response);
            tab.notice = response.error ? `Vessel refused: ${response.error}` : known ? `Receipt: ${status || 'acknowledged'}. Execution may still be pending.` : 'Outcome uncertain. Do not resend; check the receipt.';
            if (known && !response.error && ['accepted', 'queued', 'applied'].includes(status) && ['submit', 'steer', 'submit_content'].includes(op)) {
                if (tab.draft === draft) tab.draft = '';
                tab.pictures = tab.pictures.filter(picture => !pictures.includes(picture)); pictures.forEach(picture => URL.revokeObjectURL(picture.url));
            }
            return Boolean(known && !response.error && !['not_applied','unknown_after_restart'].includes(status));
        } catch { tab.notice = 'Action not confirmed. Draft retained. Check receipts before sending again.'; }
        finally { tab.busy = false; tab.stale = true; this.changed(); await this.refresh(key); }
    }
    async attach(key: string, files: File[]) {
        const tab = this.tabs.get(key); if (!tab || tab.busy) return;
        tab.busy = true; this.changed();
        try {
            for (const file of files) {
                if (!['image/png','image/jpeg','image/webp'].includes(file.type) || !file.size || tab.pictures.length >= 4 || tab.pictures.reduce((n,p) => n+p.size,0)+file.size > 2097152) throw new Error('Use PNG, JPEG or WebP; at most four pictures and 2 MiB total.');
                const bytes = new Uint8Array(await file.arrayBuffer()); let binary = '';
                for (let i=0;i<bytes.length;i+=8192) binary += String.fromCharCode(...bytes.subarray(i,i+8192));
                tab.pictures.push({id:uuid(),name:file.name || 'pasted-image',size:file.size,url:URL.createObjectURL(file),base64:btoa(binary),uploadId:uuid()});
            }
        } catch (error) { tab.notice = error instanceof Error ? error.message : 'Picture unavailable.'; }
        finally { tab.busy = false; this.changed(); }
    }
    removePicture(key: string, id: string) {
        const tab = this.tabs.get(key); if (!tab || tab.busy) return;
        tab.pictures = tab.pictures.filter(picture => { if (picture.id !== id) return true; URL.revokeObjectURL(picture.url); return false; }); this.changed();
    }
    async artifact(key: string, attachment: any) {
        const tab = this.tabs.get(key), client = tab && this.connections().get(tab.vessel)?.client;
        if (!tab || !client) throw new Error('Vessel unavailable.');
        const {imageBytes} = await import('../js/attachments.js');
        return imageBytes(client, attachment, {session_id: tab.session} as any);
    }
    async output(key: string, offset: number) {
        const tab = this.tabs.get(key); if (!tab || !this.actionable(tab)) return;
        const run = tab.snapshot.run;
        const page = await this.read(key, 'run_output', {run_id:run.run_id,offset,limit:65536});
        if (tab.snapshot.run?.run_id !== run.run_id || tab.snapshot.run?.live_text !== run.live_text || tab.snapshot.run?.partial_text !== run.partial_text || tab.snapshot.run?.live_text_offset !== run.live_text_offset || page.run_id !== run.run_id || page.offset !== offset || (page.has_more && page.next_offset <= offset)) throw new Error('Output identity changed.');
        return page;
    }
    async respond(key: string, decision: any, response: unknown) {
        const tab = this.tabs.get(key);
        if (!tab || decision.expires_at_ms <= Date.now() || decision.incarnation !== tab.incarnation || decision.run_id !== tab.snapshot?.run?.run_id) {
            if (tab) { tab.notice = 'Decision expired or voyage changed. Refresh before responding.'; this.changed(); }
            return;
        }
        return this.act(key, 'respond', {decision_id: decision.decision_id, response, expires_at_ms: Math.min(Date.now() + 60000, decision.expires_at_ms)});
    }
    async read(key: string, op: string, fields: Record<string, unknown> = {}) {
        const tab = this.tabs.get(key), connection = tab && this.connections().get(tab.vessel);
        if (!tab || !connection?.client || tab.stale) throw new Error('Wait for a fresh connected snapshot.');
        const client = connection.client, incarnation: any = tab.incarnation;
        const value = voyageResult(await client.exchange(request(op, {session_id: tab.session, ...fields})), tab.session, incarnation).result;
        if (this.closed || this.connections().get(tab.vessel)?.client !== client || tab.incarnation !== incarnation) throw new Error('Voyage connection changed.');
        return value;
    }
    async earlier(key: string) {
        const tab = this.tabs.get(key); if (!tab || !this.actionable(tab) || !tab.snapshot.message_offset) return;
        tab.busy = true; this.changed();
        const snapshot = tab.snapshot, revision = snapshot.revision;
        try {
            const end = snapshot.message_offset, start = Math.max(0, end - 50); let offset = start;
            const messages: any[] = [];
            while (offset < end) {
                const page = await this.read(key, 'history', {offset, limit: end - offset, expected_revision: revision});
                if (page.revision !== revision || !Number.isSafeInteger(page.next_offset) || page.next_offset <= offset || page.next_offset > end || !Array.isArray(page.messages)) throw new Error('Invalid history continuation.');
                messages.push(...page.messages); offset = page.next_offset;
            }
            if (tab.snapshot !== snapshot) throw new Error('History changed. Refresh before loading earlier messages.');
            tab.snapshot = {...snapshot, messages: [...messages, ...snapshot.messages], message_offset: start};
        } catch (error) { tab.notice = error instanceof Error ? error.message : 'History unavailable.'; }
        finally { tab.busy = false; this.changed(); }
    }
    async expand(key: string, index: number) {
        const tab = this.tabs.get(key); if (!tab || !this.actionable(tab)) return;
        tab.busy = true; this.changed(); const revision = tab.snapshot.revision;
        try {
            let offset = 0, text = '', page;
            do {
                page = await this.read(key, 'message_chunk', {index, offset, limit: 65536, expected_revision: revision});
                if (page.total_bytes > 4*1024*1024 || (page.has_more && (!Number.isSafeInteger(page.next_offset) || page.next_offset <= offset))) throw new Error('Message exceeds expansion limit or has invalid continuation.');
                if (typeof page.data !== 'string' || new TextEncoder().encode(text).length + new TextEncoder().encode(page.data).length > 4*1024*1024) throw new Error('Message exceeds browser expansion limit.');
                text += page.data; offset = page.next_offset;
            } while (page.has_more);
            if (tab.snapshot.revision !== revision) throw new Error('History changed.');
            const expanded = {...JSON.parse(text), message_index: index, projection_truncated: false};
            tab.snapshot = {...tab.snapshot, messages: tab.snapshot.messages.map((message: any) => message.message_index === index ? expanded : message)};
        } catch (error) { tab.notice = error instanceof Error ? error.message : 'Message unavailable.'; }
        finally { tab.busy = false; this.changed(); }
    }
    async reconcile(key: string) {
        const tab = this.tabs.get(key); if (!tab || tab.busy) return;
        const connection = this.connections().get(tab.vessel); if (!connection?.client) return;
        tab.busy = true; this.changed();
        try {
            for (const entry of this.pending(tab)) {
                const response = await connection.client.exchange(request('receipt', {session_id: tab.session, command_id: entry.command_id}));
                if (resolved(response, entry.command_id, tab.session, true) && receiptStatus(response) !== 'unknown_after_restart') {
                    connection.journal.settle(entry.command_id);
                    tab.notice = `Receipt ${entry.command_id}: ${receiptStatus(response)}. Review retained draft before sending.`;
                } else tab.notice = `Receipt ${entry.command_id} remains uncertain. No action replayed.`;
            }
        } catch { tab.notice = 'Receipt unavailable. No action replayed.'; }
        finally { tab.busy = false; this.changed(); }
    }
    close() { this.tabs.forEach(tab => tab.pictures.forEach(picture => URL.revokeObjectURL(picture.url))); this.closed = true; this.streams.forEach(stream => stream.stop()); this.streams.clear(); this.listeners.clear(); }
}
