// Covers framework-generated and dynamically inserted icon-only controls too.
export function installButtonTooltips(doc = document) {
    const tip = doc.createElement('div');
    tip.id = 'helm-button-tooltip';
    tip.role = 'tooltip';
    tip.hidden = true;
    tip.setAttribute('popover', 'manual');
    Object.assign(tip.style, {position:'fixed', margin:'0', inset:'auto', zIndex:'2147483647', maxWidth:'18rem', padding:'6px 10px', border:'1px solid #71717a', borderRadius:'6px', background:'#18181b', color:'#fafafa', font:'12px system-ui', pointerEvents:'none'});
    doc.body.append(tip);
    let active = null;
    function hide() {
        if (active) {
            const ids = (active.getAttribute('aria-describedby') || '').split(/\s+/).filter(id => id && id !== tip.id);
            if (ids.length) active.setAttribute('aria-describedby', ids.join(' '));
            else active.removeAttribute('aria-describedby');
        }
        if (!tip.hidden && tip.hidePopover) tip.hidePopover();
        tip.hidden = true;
        active = null;
    }
    function show(event) {
        const button = event.target.closest?.('button, [role="button"], a[data-flux-button]');
        if (!button || button.closest('[data-flux-tooltip]')) return;
        const clone = button.cloneNode(true);
        clone.querySelectorAll('svg, [aria-hidden="true"], .sr-only').forEach(node => node.remove());
        if (clone.textContent.trim() && !button.matches('[data-flux-profile]')) return;
        const label = button.getAttribute('title') || button.getAttribute('aria-label');
        if (!label) return;
        hide();
        active = button;
        tip.textContent = label;
        const ids = (button.getAttribute('aria-describedby') || '').split(/\s+/).filter(Boolean);
        button.setAttribute('aria-describedby', [...ids, tip.id].join(' '));
        tip.hidden = false;
        if (tip.showPopover) tip.showPopover();
        const rect = button.getBoundingClientRect();
        const bounds = tip.getBoundingClientRect();
        const win = doc.defaultView;
        tip.style.left = `${Math.max(8, Math.min(rect.left, win.innerWidth - bounds.width - 8))}px`;
        tip.style.top = `${rect.top >= bounds.height + 12 ? rect.top - bounds.height - 6 : rect.bottom + 6}px`;
    }
    doc.addEventListener('pointerover', show);
    doc.addEventListener('focusin', show);
    doc.addEventListener('pointerout', hide);
    doc.addEventListener('focusout', hide);
    doc.addEventListener('click', hide);
    doc.addEventListener('keydown', event => { if (event.key === 'Escape') hide(); });
    doc.addEventListener('scroll', hide, true);
    doc.defaultView.addEventListener('resize', hide);
}

if (typeof document !== 'undefined') {
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', () => installButtonTooltips(), {once:true});
    else installButtonTooltips();
}
