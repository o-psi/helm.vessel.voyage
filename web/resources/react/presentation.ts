// Mirrors existing console presentation; metadata is never execution authority.
// Presentation only: never use catalogue metadata to authorize runtime actions.
export function cardStatus(item: any, connected: boolean, snapshot: any = null) {
    if (!connected) return {label: 'Offline', tone: 'muted', animated: false};
    const state = item.state;
    if (state === 'cleanup_unconfirmed') return {label: 'Cleanup pending', tone: 'warning', animated: false};
    if (['unavailable', 'stopped', 'relinquished', 'suspended'].includes(state)) {
        return {label: state[0].toUpperCase() + state.slice(1), tone: state === 'unavailable' ? 'error' : 'muted', animated: false};
    }
    if (!snapshot && item.catalogue?.stale) return {label: 'Cached', tone: 'muted', animated: false};
    const run = snapshot ? snapshot.run?.state : item.catalogue?.summary?.run_state;
    const label = run || state || 'unknown';
    const tone = ['running', 'starting', 'live'].includes(label) ? 'active'
        : ['cancelling', 'blocked', 'waiting'].includes(label) ? 'warning'
        : ['failed', 'error'].includes(label) ? 'error'
        : ['completed', 'succeeded'].includes(label) ? 'success' : 'muted';
    return {label: label[0].toUpperCase() + label.slice(1).replaceAll('_', ' '), tone,
        animated: ['running', 'starting', 'live', 'cancelling'].includes(label)};
}

// Catalogue metadata is display-only, never evidence of runtime liveness.
export function voyageActivity(voyage: any) {
    const summary = voyage.catalogue?.summary;
    for (const [kind, value] of [['Last turn ended', summary?.last_turn_end], ['Created', summary?.created_at]]) {
        const ms = typeof value === 'string' ? Date.parse(value) : NaN;
        if (Number.isFinite(ms)) return {kind, ms, iso: new Date(ms).toISOString()};
    }
    return null;
}

export function activityLabel(activity: any, now = new Date()) {
    if (!activity) return 'No timestamp';
    const date = new Date(activity.ms);
    if (date.toDateString() === now.toDateString()) return date.toLocaleTimeString(undefined, {hour: 'numeric', minute: '2-digit'});
    return date.toLocaleDateString(undefined, {month: 'short', day: 'numeric', ...(date.getFullYear() !== now.getFullYear() ? {year: 'numeric'} : {})});
}

// Ranking is presentation only. Prefer a fresh open-view snapshot over cached catalogue state.
export function activeVoyage(voyage: any, snapshot?: any) {
    if (voyage.catalogue?.summary?.archived || voyage.catalogue?.summary?.deleted || snapshot?.lifecycle?.archived || snapshot?.lifecycle?.deleted) return false;
    if (['stopped','unavailable','relinquished','suspended'].includes(voyage.state)) return false;
    const state = snapshot ? snapshot.run?.state : voyage.catalogue?.summary?.run_state;
    return ['starting','running','cancelling','waiting','blocked'].includes(state);
}

export function voyageList(connections: any[], query = '', snapshotFor: (connection: any, voyage: any) => any = () => null) {
    const search = query.trim().toLocaleLowerCase();
    return [...connections].flatMap(connection => connection.voyages.map((voyage: any) => ({
        ...voyage, connection, activity: voyageActivity(voyage), active: activeVoyage(voyage, snapshotFor(connection, voyage)),
    }))).filter(v => `${v.name || ''} ${v.session_id} ${v.connection.name}`.toLocaleLowerCase().includes(search))
        .sort((a, b) => Number(b.active) - Number(a.active) || (b.activity?.ms ?? -Infinity) - (a.activity?.ms ?? -Infinity)
            || a.connection.id.localeCompare(b.connection.id) || a.session_id.localeCompare(b.session_id));
}
