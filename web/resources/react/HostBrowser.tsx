import React, {useEffect, useId, useRef, useState} from 'react';
import {mountHostBrowser} from '../js/host-browser.js';
import type {Tab} from './workspace';

// Only the selected Workspace tab and its exact live socket may own a viewer.
export function HostBrowser({tab, client}: {tab: Tab; client: any}) {
    const [open, setOpen] = useState(false);
    const root = useRef<HTMLDivElement>(null);
    const latest = useRef(tab);
    latest.current = tab;
    const id = useId();
    const ready = Boolean(client && !tab.stale && tab.incarnation && Number.isSafeInteger(tab.snapshot?.revision) && tab.snapshot.revision >= 0);
    useEffect(() => {
        if (!open || !ready || !root.current) return;
        const viewer = mountHostBrowser(root.current, {
            client, sessionId: tab.session, incarnation: tab.incarnation,
            context: () => ({revision: latest.current.snapshot.revision}),
            onClose: () => setOpen(false),
        });
        // Shared disposal detaches this viewer, never closes the host browser.
        return () => viewer.dispose();
    }, [open, ready, client, tab.vessel, tab.session, tab.incarnation]);
    return <section className="react-host-browser" aria-label="Task browser">
        <div className="task-browser-heading"><span>{tab.title}</span><button type="button" aria-expanded={open} aria-controls={id} onClick={() => setOpen(value => !value)}>Browser</button></div>
        {open && <div id={id} role="region" aria-label="Host browser">
            {!ready && <p role="status">Browser detached. Waiting for a current Vessel connection and voyage snapshot.</p>}
            <div ref={root}/>
        </div>}
    </section>;
}
