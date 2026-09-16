// Display-only public observations. Never turn a preview into an executable call.
const clean = value => String(value ?? '').replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/g, '');
const bounded = (value, limit) => {
    const bytes = new TextEncoder().encode(clean(value));
    return {text:new TextDecoder('utf-8', {fatal:false}).decode(bytes.subarray(0,limit)).replace(/\ufffd$/, ''), truncated:bytes.length > limit};
};

// Mirrors the native ToolPreview/ReasoningPreview display limits. Identity is
// attempt + index, not the provider call ID (which can arrive in fragments).
export function renderPreviews(root, run, messages = []) {
    const canonical = new Set(messages.flatMap(message => (message.tool_calls || []).map(call => call.id)));
    const rows = [];
    for (const preview of (run?.tool_previews || []).slice(0,32)) {
        if (preview.call_id && canonical.has(preview.call_id)) continue;
        const value = bounded(preview.arguments, 16384);
        rows.push({key:`tool:${preview.attempt_id}:${preview.index}`, title:`${clean(preview.name) || 'Tool call'} · streaming · provisional`, text:value.text, truncated:value.truncated || preview.truncated});
    }
    for (const preview of (run?.reasoning_previews || []).slice(0,32)) {
        const value = bounded(preview.text, 16384);
        rows.push({key:`reasoning:${preview.attempt_id}:${preview.index}:${preview.kind}:${Boolean(preview.finalized)}`, title:`${preview.kind === 'summary' ? 'Reasoning summary' : 'Provider thinking'} · ${preview.finalized ? 'finalized disclosure' : 'streaming · provisional'}`, text:value.text, truncated:value.truncated || preview.truncated});
    }
    const old = new Map([...root.children].map(node => [node.dataset.previewKey,node]));
    for (const row of rows) {
        let node = old.get(row.key);
        if (!node) {
            node = document.createElement('details'); node.dataset.previewKey = row.key;
            node.className = 'my-4 rounded-xl border border-zinc-200 p-3 dark:border-zinc-700';
            node.append(document.createElement('summary'), document.createElement('pre'), document.createElement('p'));
            node.children[1].className = 'mt-3 max-h-80 overflow-auto whitespace-pre-wrap break-words text-sm';
        }
        node.children[0].textContent = row.title;
        node.children[1].textContent = row.text;
        node.children[2].textContent = row.truncated ? '… preview bounded; more content may be omitted' : '';
        root.append(node); old.delete(row.key);
    }
    for (const node of old.values()) node.remove();
    root.hidden = !rows.length;
}

export class ConversationStream {
    seed(snapshot, incarnation) {
        this.session = snapshot.session_id; this.incarnation = incarnation;
        this.cursor = snapshot.observation_cursor;
        this.valid = Number.isSafeInteger(this.cursor) && this.cursor >= 0;
    }
    accept(event) {
        const fail = () => { this.valid = false; return 'resync'; };
        if (!this.valid || event.protocol !== 1 || event.session_id !== this.session || event.incarnation !== this.incarnation || event.error != null || event.outcome_unknown !== false) return fail();
        const page = event.result;
        if (!page || page.projection !== 'public-v1' || page.replay_gap || !Number.isSafeInteger(page.cursor) || !Number.isSafeInteger(page.latest_cursor) || page.cursor > page.latest_cursor || !Array.isArray(page.events)) return fail();
        if (page.cursor < this.cursor) return fail();
        if (page.cursor === this.cursor) return 'duplicate';
        // SQLite observation cursors are globally allocated, not session-contiguous.
        let cursor = this.cursor;
        for (const item of page.events) {
            if (!Number.isSafeInteger(item.cursor) || item.cursor <= cursor || item.cursor > page.cursor || item.session_id !== this.session) return fail();
            cursor = item.cursor;
        }
        if (cursor !== page.cursor) return fail();
        this.cursor = cursor;
        // Existing public-v1 events are invalidations, not appendable text.
        // Unknown kinds also require canonical recovery rather than guessing.
        return 'refresh';
    }
}
