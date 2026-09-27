// Keep ordinary links as a fallback, but open management in place when Flux is ready.
export function openVesselManager(event, document, flux) {
    const link = event.target.closest?.('a[href]');
    const modal = document.querySelector('[data-connections-url]');
    if (!link || !modal || event.defaultPrevented || event.button !== 0
        || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey
        || link.target === '_blank' || link.hasAttribute('download')
        || link.href !== modal.dataset.connectionsUrl || typeof flux?.modal !== 'function') return;
    event.preventDefault();
    flux.modal('manage-vessels').show();
}

if (typeof document !== 'undefined') {
    document.addEventListener('click', event => openVesselManager(event, document, window.Flux));
}
