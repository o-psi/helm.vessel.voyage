// Presentation only: never use catalogue metadata to authorize runtime actions.
export function cardStatus(item, connected, snapshot = null) {
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

export function decorateCard(control, status) {
    control.dataset.statusTone = status.tone;
    control.toggleAttribute('data-animated', status.animated);
    let detail = control.querySelector('[data-card-status]');
    if (!detail) {
        detail = document.createElement('span');
        detail.dataset.cardStatus = '';
        const indicator = document.createElement('span');
        indicator.dataset.cardIndicator = '';
        indicator.setAttribute('aria-hidden', 'true');
        detail.append(indicator, document.createTextNode(''));
        control.querySelector('[data-content]').append(detail);
    }
    detail.lastChild.textContent = status.label;
}
