import React, {useEffect, useId, useRef, useState} from 'react';
import {mountHostBrowser} from '../js/host-browser.js';
import type {Tab} from './workspace';

// Message metadata, not prose: mentioning a tool is not browser activity.
export function browserActivity(snapshot: any): boolean {
    const calls = new Set<string>();
    const named = (name: unknown) => typeof name === 'string' && /^(?:functions\.)?host_browser$/.test(name);
    for (const message of snapshot?.messages || []) {
        for (const call of message.tool_calls || []) {
            if (named(call.name ?? call.function?.name)) calls.add(call.id);
        }
        if (named(message.name) && ['tool', 'function'].includes(message.role)) return true;
    }
    return calls.size > 0 || (snapshot?.run?.tool_previews || []).some((call: any) => named(call.name));
}

// Only the selected voyage and its exact live socket may own a viewer.
export function HostBrowser({tab, client}: {tab: Tab; client: any}) {
    const [open, setOpen] = useState(false);
    const [expanded, setExpanded] = useState(false);
    const root = useRef<HTMLDivElement>(null), panel = useRef<HTMLElement>(null), toggle = useRef<HTMLButtonElement>(null);
    const latest = useRef(tab);
    latest.current = tab;
    const id = useId();
    const activity = browserActivity(tab.snapshot);
    const ready = Boolean(client);
    const close = () => { setExpanded(false);setOpen(false); toggle.current?.focus(); };
    useEffect(() => {
        if (!open || !ready || !root.current) return;
        const viewer = mountHostBrowser(root.current, {
            client, sessionId: tab.session, incarnation: tab.incarnation,
            context: () => ({revision: latest.current.snapshot?.revision, refresh:latest.current.stale || !latest.current.incarnation}),
            onClose: close, externalClose:true,
        });
        // Shared viewer auto-connects on mount; disposal detaches, never closes the browser.
        return () => {viewer.dispose();};
    }, [open, ready, client, tab.vessel, tab.session, tab.incarnation]);
    useEffect(() => {
        if (!open) return;
        panel.current?.focus();
        const mobile=window.matchMedia?.('(max-width: 1000px)').matches;
        const background=mobile?[...document.querySelectorAll<HTMLElement>('.conversation,.sidebar')]:[];
        const previous=background.map(node=>node.inert);background.forEach(node=>node.inert=true);
        if(mobile){panel.current?.setAttribute('role','dialog');panel.current?.setAttribute('aria-modal','true');}
        const key = (event: KeyboardEvent) => {
            if (event.defaultPrevented) return;
            if (event.key === 'Escape') { event.preventDefault(); close(); }
            if (event.key !== 'Tab' || !window.matchMedia?.('(max-width: 1000px)').matches) return;
            const nodes = [...panel.current!.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), summary, [tabindex="0"]')].filter(node => !node.hidden && !node.closest('[hidden]') && !node.closest('dialog:not([open])') && (!node.closest('details:not([open])') || node.tagName==='SUMMARY') && node.getClientRects().length);
            const first = nodes[0], last = nodes.at(-1);
            if (event.shiftKey && (document.activeElement === first || document.activeElement === panel.current)) { event.preventDefault(); last?.focus(); }
            else if (!event.shiftKey && (document.activeElement === last || document.activeElement === panel.current)) { event.preventDefault(); first?.focus(); }
        };
        document.addEventListener('keydown', key);
        return () => {document.removeEventListener('keydown', key);background.forEach((node,i)=>node.inert=previous[i]);};
    }, [open]);
    useEffect(() => {
        if (!open || !expanded || window.matchMedia?.('(max-width: 1000px)').matches) return;
        const conversation = panel.current?.closest('.voyage-workspace')?.querySelector<HTMLElement>(':scope > .conversation');
        if (!conversation) return;
        const previous = conversation.inert; conversation.inert = true;
        return () => {conversation.inert = previous;};
    }, [open, expanded]);
    return <div className="react-host-browser">
        <button ref={toggle} className="task-browser-action" type="button" aria-expanded={open} aria-controls={id} aria-describedby={activity ? `${id}-activity` : undefined} title={`Browser · ${tab.title}`} onClick={() => open ? close() : setOpen(true)}>
            Browser{activity && <span className="browser-activity" aria-hidden="true"/>}
        </button>
        {activity && <span id={`${id}-activity`} className="sr-only">Browser activity in this voyage</span>}
        {open && <aside ref={panel} tabIndex={-1} id={id} className={`task-browser-panel${expanded?' expanded':''}`} aria-labelledby={`${id}-title`}>
            <header className="task-browser-heading"><div><h2 id={`${id}-title`}>Browser</h2><span>{tab.title}</span></div><div className="task-browser-heading-actions"><button className="expand-browser" type="button" hidden={window.matchMedia?.('(max-width: 1000px)').matches} aria-pressed={expanded} onClick={() => setExpanded(value => !value)}>{expanded?'Show chat':'Expand browser'}</button><button type="button" aria-label="Close browser viewer" title="Close viewer; keep browser running" onClick={close}>Close ×</button></div></header>
            {!ready && <p role="status">Vessel disconnected. The browser will reconnect when this Vessel connection returns.</p>}
            <div className="task-browser-content" ref={root}/>
        </aside>}
    </div>;
}
