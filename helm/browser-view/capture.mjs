const MAX_BYTES = 2 * 1024 * 1024;

/** Local viewer-image editor, not a semantic browser tool or message sender.
 * Parent: include capture.css; call open() only from the Capture user gesture.
 * canCapture() must return true only while the visible stream is eligible.
 * Call reset() on privacy, control, stream, tab or voyage changes (even if still
 * eligible), and dispose() on unmount. onCapture(File) adds to draft, never sends.
 * Confirmation is deliberately privacy-explicit in every mode: no mode inference.
 */
export function mountCapture({root, video, canCapture, onCapture}) {
    const doc = root.ownerDocument, win = doc.defaultView;
    const dialog = doc.createElement('dialog');
    dialog.className = 'helm-capture';
    dialog.setAttribute('aria-label', 'Capture viewer image');
    dialog.innerHTML = `<header><h2>Capture viewer image</h2><p>A frozen image of the visible viewer, not a semantic browser-tool result.</p></header>
        <div class="helm-capture-stage"><canvas tabindex="0" aria-label="Rectangle annotation canvas. Arrow keys move the cursor; Space starts a rectangle; Enter finishes it; Escape cancels it."></canvas></div>
        <p class="helm-capture-help">Drag to draw a rectangle. Keyboard: arrows move, Space starts, Enter finishes, Escape cancels.</p>
        <p class="helm-capture-status" role="status" aria-live="polite"></p>
        <label class="helm-capture-consent"><input type="checkbox"> I understand this captured image leaves the private view and is added to the message draft. It is not sent until I send the message.</label>
        <footer><button type="button" data-action="clear">Clear annotations</button><span></span><button type="button" data-action="cancel">Cancel</button><button type="button" data-action="export"></button></footer>`;
    root.append(dialog);
    const canvas = dialog.querySelector('canvas'), frozen = doc.createElement('canvas');
    const ctx = canvas.getContext('2d'), source = frozen.getContext('2d');
    const clear = dialog.querySelector('[data-action="clear"]'), cancel = dialog.querySelector('[data-action="cancel"]');
    const save = dialog.querySelector('[data-action="export"]'), consent = dialog.querySelector('input');
    const status = dialog.querySelector('[role="status"]');
    const attach = typeof onCapture === 'function';
    dialog.querySelector('label').hidden = !attach;
    save.textContent = attach ? 'Add to message' : 'Download capture';
    let disposed = false, active = false, busy = false, generation = 0, previousFocus;
    let rectangles = [], start = null, cursor = {x: 0, y: 0}, pointer = null;
    const urls = new Set();
    const eligible = () => { try { return canCapture() === true; } catch { return false; } };
    const update = () => { save.disabled = !active || busy || (attach && !consent.checked); clear.disabled = !active || busy; consent.disabled = busy; };
    const release = () => {
        const held = pointer; pointer = null;
        if (held !== null) { try { canvas.releasePointerCapture(held); } catch { /* already released */ } }
    };
    const reset = () => {
        generation++; active = false; busy = false; release(); rectangles = []; start = null; cursor = {x: 0, y: 0};
        canvas.width = canvas.height = frozen.width = frozen.height = 0;
        for (const url of urls) win.URL.revokeObjectURL(url);
        urls.clear(); consent.checked = false; status.textContent = ''; update();
        if (dialog.open) dialog.close();
        if (previousFocus?.isConnected) previousFocus.focus();
        previousFocus = null;
    };
    const paint = () => {
        ctx.clearRect(0, 0, canvas.width, canvas.height); ctx.drawImage(frozen, 0, 0);
        ctx.lineWidth = Math.max(2, canvas.width / 500);
        const box = (a, b) => {
            ctx.strokeStyle = '#111827'; ctx.strokeRect(a.x - 1, a.y - 1, b.x - a.x + 2, b.y - a.y + 2);
            ctx.strokeStyle = '#fbbf24'; ctx.strokeRect(a.x, a.y, b.x - a.x, b.y - a.y);
        };
        for (const [a, b] of rectangles) box(a, b);
        if (start) box(start, cursor);
    };
    const finish = () => {
        if (start && cursor.x !== start.x && cursor.y !== start.y) rectangles.push([start, {...cursor}]);
        start = null; paint();
    };
    const point = event => {
        const bounds = canvas.getBoundingClientRect();
        return {x: Math.round(Math.max(0, Math.min(canvas.width, (event.clientX - bounds.left) * canvas.width / bounds.width))),
            y: Math.round(Math.max(0, Math.min(canvas.height, (event.clientY - bounds.top) * canvas.height / bounds.height)))};
    };
    const check = () => {
        if (!active || busy) return false;
        if (!eligible()) { reset(); return false; }
        return true;
    };
    canvas.addEventListener('pointerdown', event => {
        if (!check() || event.button !== 0 || pointer !== null) return;
        event.preventDefault(); canvas.focus(); cursor = point(event); start = {...cursor}; pointer = event.pointerId;
        try { canvas.setPointerCapture(pointer); } catch { start = null; pointer = null; }
    });
    canvas.addEventListener('pointermove', event => {
        if (!check() || pointer !== event.pointerId) return;
        cursor = point(event); paint();
    });
    canvas.addEventListener('pointerup', event => {
        if (!check() || pointer !== event.pointerId) return;
        cursor = point(event); release(); finish();
    });
    const abandon = () => { release(); start = null; if (active && !busy) paint(); };
    canvas.addEventListener('pointercancel', abandon);
    canvas.addEventListener('lostpointercapture', () => { if (pointer !== null) abandon(); });
    canvas.addEventListener('keydown', event => {
        if (!check()) return;
        if (event.key === 'Escape' && start) { event.preventDefault(); event.stopPropagation(); abandon(); return; }
        if (pointer !== null) return;
        const directions = {ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1]};
        if (directions[event.key]) {
            event.preventDefault(); const [x, y] = directions[event.key], step = event.shiftKey ? 1 : 10;
            cursor = {x: Math.max(0, Math.min(canvas.width, cursor.x + x * step)), y: Math.max(0, Math.min(canvas.height, cursor.y + y * step))};
            paint(); status.textContent = `Annotation cursor: ${cursor.x}, ${cursor.y}.`;
        } else if (event.key === ' ' || event.key === 'Enter') {
            event.preventDefault(); if (start) finish(); else start = {...cursor};
            status.textContent = start ? 'Rectangle started. Move with arrows, then Enter to finish.' : 'Rectangle finished.';
        }
    });
    clear.addEventListener('click', () => { if (check()) { release(); start = null; rectangles = []; paint(); status.textContent = 'Annotations cleared. Original capture preserved.'; } });
    consent.addEventListener('change', update);
    cancel.addEventListener('click', reset);
    dialog.addEventListener('cancel', event => { event.preventDefault(); reset(); });
    dialog.addEventListener('close', () => { if (active) reset(); });
    save.addEventListener('click', async () => {
        if (!check() || (attach && !consent.checked)) return;
        start = null; release(); paint(); busy = true; update(); status.textContent = 'Preparing PNG…';
        const token = generation;
        try {
            const blob = await new Promise(resolve => canvas.toBlob(resolve, 'image/png'));
            if (token !== generation || disposed) return;
            if (!eligible()) { reset(); return; }
            if (!blob || blob.type !== 'image/png' || blob.size === 0 || blob.size > MAX_BYTES) {
                throw new Error('size');
            }
            if (attach) {
                const file = new win.File([blob], 'helm-viewer-capture.png', {type: 'image/png'});
                await onCapture(file);
            } else {
                const url = win.URL.createObjectURL(blob); urls.add(url);
                const link = doc.createElement('a'); link.href = url; link.download = 'helm-viewer-capture.png';
                dialog.append(link); link.click(); link.remove();
            }
            if (token === generation) reset();
        } catch {
            if (token !== generation || disposed) return;
            busy = false; update();
            status.textContent = 'Capture could not be added or downloaded. PNG must be at most 2 MiB. Nothing was sent by this viewer.';
        }
    });
    update();
    return {
        open() {
            if (disposed || active || !eligible()) return false;
            if (!ctx || !source || video.readyState < 2 || !(video.videoWidth > 0 && video.videoHeight > 0)) return false;
            previousFocus = doc.activeElement;
            try {
                const scale = Math.min(1, 4096 / video.videoWidth, 4096 / video.videoHeight, Math.sqrt(8_000_000 / (video.videoWidth * video.videoHeight)));
                canvas.width = frozen.width = Math.max(1, Math.floor(video.videoWidth * scale));
                canvas.height = frozen.height = Math.max(1, Math.floor(video.videoHeight * scale));
                source.drawImage(video, 0, 0, frozen.width, frozen.height);
                active = true; cursor = {x: Math.round(canvas.width / 2), y: Math.round(canvas.height / 2)};
                paint(); update(); dialog.showModal(); cancel.focus(); return true;
            } catch { reset(); return false; }
        },
        reset,
        dispose() { if (disposed) return; reset(); disposed = true; dialog.remove(); },
    };
}
