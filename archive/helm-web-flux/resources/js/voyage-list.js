// Catalogue metadata is display-only, never evidence of runtime liveness.
export function voyageActivity(voyage) {
    const summary = voyage.catalogue?.summary;
    for (const [kind, value] of [['Last turn ended', summary?.last_turn_end], ['Created', summary?.created_at]]) {
        const ms = typeof value === 'string' ? Date.parse(value) : NaN;
        if (Number.isFinite(ms)) return {kind, ms, iso: new Date(ms).toISOString()};
    }
    return null;
}

export function activityLabel(activity, now = new Date()) {
    if (!activity) return 'No timestamp';
    const date = new Date(activity.ms);
    if (date.toDateString() === now.toDateString()) return date.toLocaleTimeString(undefined, {hour: 'numeric', minute: '2-digit'});
    return date.toLocaleDateString(undefined, {month: 'short', day: 'numeric', ...(date.getFullYear() !== now.getFullYear() ? {year: 'numeric'} : {})});
}

export function voyageList(connections, query = '') {
    const search = query.trim().toLocaleLowerCase();
    return [...connections].flatMap(connection => connection.voyages.map(voyage => ({
        ...voyage, connection, activity: voyageActivity(voyage),
    }))).filter(v => `${v.name || ''} ${v.session_id} ${v.connection.name}`.toLocaleLowerCase().includes(search))
        .sort((a, b) => (b.activity?.ms ?? -Infinity) - (a.activity?.ms ?? -Infinity)
            || a.connection.id.localeCompare(b.connection.id) || a.session_id.localeCompare(b.session_id));
}
