// One follow state for history, provisional output, and delayed layout changes.
export function conversationScroll(viewport, jump) {
    const win = viewport.ownerDocument.defaultView;
    const content = viewport.ownerDocument.createElement('div');
    content.dataset.conversationContent = '';
    content.append(...viewport.childNodes);
    viewport.append(content);
    jump.type = 'button'; jump.textContent = 'Jump to latest ↓';
    jump.classList.add('mx-auto', 'my-2', 'shrink-0');
    jump.hidden = true;
    viewport.after(jump);
    let following = true, frame = null, disposed = false, lastTop = viewport.scrollTop;
    const bottom = () => Math.max(0, viewport.scrollHeight - viewport.clientHeight);
    const nearBottom = () => bottom() - viewport.scrollTop <= 80;
    const setFollowing = value => { following = value; jump.hidden = value; };
    const schedule = () => {
        if (disposed || frame !== null) return;
        frame = win.requestAnimationFrame(() => {
            frame = null;
            if (!following || disposed) return;
            viewport.scrollTop = bottom();
            lastTop = viewport.scrollTop;
        });
    };
    const scroll = () => {
        const top = viewport.scrollTop;
        // Content growth alone must not turn following off. Clamping after a
        // content shrink isn't an upward user scroll either.
        if (top < Math.min(lastTop, bottom()) - 1) setFollowing(false);
        else if (nearBottom()) setFollowing(true);
        lastTop = top;
        if (following) schedule();
    };
    const wheel = event => { if (event.deltaY < 0) setFollowing(false); };
    const key = event => {
        if (event.target !== viewport) return;
        if (['ArrowUp','PageUp','Home'].includes(event.key) || (event.key === ' ' && event.shiftKey)) setFollowing(false);
    };
    const reset = () => { setFollowing(true); lastTop = viewport.scrollTop; schedule(); };
    viewport.addEventListener('scroll', scroll, {passive:true});
    viewport.addEventListener('wheel', wheel, {passive:true});
    viewport.addEventListener('keydown', key);
    jump.addEventListener('click', reset);
    const resize = win.ResizeObserver ? new win.ResizeObserver(schedule) : null;
    resize?.observe(content); resize?.observe(viewport);
    // Also schedules updates in hosts without ResizeObserver; load captures
    // late image sizing, while mutations include provisional streaming output.
    const mutations = new win.MutationObserver(schedule);
    mutations.observe(content, {subtree:true, childList:true, characterData:true, attributes:true});
    content.addEventListener('load', schedule, true);
    win.addEventListener('resize', schedule);
    schedule();
    return {
        reset,
        preservePrepend(render) {
            setFollowing(false);
            const height = viewport.scrollHeight, top = viewport.scrollTop;
            render();
            viewport.scrollTop = top + viewport.scrollHeight - height;
            lastTop = viewport.scrollTop;
        },
        dispose() {
            disposed = true;
            if (frame !== null) win.cancelAnimationFrame(frame);
            resize?.disconnect(); mutations.disconnect();
            viewport.removeEventListener('scroll', scroll);
            viewport.removeEventListener('wheel', wheel);
            viewport.removeEventListener('keydown', key);
            content.removeEventListener('load', schedule, true);
            win.removeEventListener('resize', schedule);
            jump.removeEventListener('click', reset); jump.remove();
        },
    };
}
