// Web presentation of the saved action facts used by the TUI transcript.
export function actionDescription(call) {
    const name = call.function?.name || call.name || 'Tool';
    let args = call.function?.arguments ?? call.arguments ?? {};
    if (typeof args === 'string') { try { args = JSON.parse(args); } catch { args = {}; } }
    const path = args.path || '.';
    switch (name) {
        case 'shell': return `Run ${args.command || ''}`;
        case 'read_file': return `Read ${path}`;
        case 'write_file': return `Write ${path} · ${String(args.content || '').split('\n').filter((v,i,a) => i < a.length-1 || v).length} lines`;
        case 'apply_patch': { const lines = String(args.patch || '').split('\n'); return `Edit ${path} · +${lines.filter(l=>l.startsWith('+')&&!l.startsWith('+++')).length}/−${lines.filter(l=>l.startsWith('-')&&!l.startsWith('---')).length} lines`; }
        case 'search_files': return `Search ${path} for “${args.query || ''}”${args.glob ? ` in ${args.glob}` : ''}`;
        case 'list_directory': return `List ${path}`;
        default: return `${name}${args.action ? ` · ${args.action}` : ''}`;
    }
}
export function actionStatus(call,result,active,decisions) {
    if (!result) return active ? decisions ? 'Awaiting approval' : 'Working' : 'Unconfirmed';
    const o = result.tool_outcome;
    if (o) {
        const labels = {policy_refused:'Refused',approval_denied:'Approval denied',approval_expired:'Approval expired',approval_invalidated:'Approval invalidated',approval_unavailable:'Approval unavailable',cancelled:'Cancelled',unknown:'Unconfirmed',execution_error:'Execution error'};
        if (labels[o.execution]) return labels[o.execution];
        if (o.command?.status === 'signalled') return 'Command signalled';
        if (o.command?.status === 'exited' && o.command.code !== 0) return `Command failed (exit ${o.command.code})`;
        if (o.incomplete) return 'Output incomplete';
    }
    if (result.tool_success === false) return 'Failed';
    if ((call.name || call.function?.name) === 'shell' && /^exit: (?!0(?:\n|$))-?\d+/.test(result.content || '')) return 'Failed';
    return result.tool_success === true ? 'Done' : 'Received';
}
export function actionDuration(result) {
    if (!result) return '';
    const ms = result.tool_outcome?.elapsed_ms;
    if (!Number.isFinite(ms)) return 'timing unavailable';
    return ms < 1000 ? `${ms} ms` : ms < 60000 ? `${(Math.floor(ms/100)/10).toFixed(1)} s` : `${Math.floor(ms/60000)}m ${String(Math.floor(ms/1000)%60).padStart(2,'0')}s`;
}
