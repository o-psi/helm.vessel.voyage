import React, {useEffect, useRef, useState} from 'react';

type Vessel = {id: string; name: string; vessel_id: string; endpoint?: string};
type Pending = {id: string; name: string};
type Bootstrap = {
    vessels: Vessel[]; pairings?: Pending[]; principalId?: string;
    connectionStatus?: string | null; connectionError?: string | null;
    connectionForm?: string | null;
};
type Status = {connected: boolean; status: string};
type Screen = 'list' | 'add' | 'guide' | 'details';
type Method = 'invitation' | 'credential';

const guideUrl = 'https://github.com/o-psi/helm.vessel.voyage/blob/main/docs/getting-started-web.md';

function endpointHost(endpoint?: string): string {
    if (!endpoint) return 'Saved Vessel';
    try { return new URL(endpoint).host || endpoint; }
    catch { return endpoint; }
}

function bootstrapFrom(html: string): Bootstrap {
    const page = new DOMParser().parseFromString(html, 'text/html');
    const value = page.querySelector<HTMLElement>('#helm-react')?.dataset.bootstrap;
    if (!value) throw new Error('Connection status could not be read. Reload to inspect it before trying again.');
    const data = JSON.parse(value) as Bootstrap;
    if (!Array.isArray(data.vessels) || !Array.isArray(data.pairings)) {
        throw new Error('Connection status was incomplete. Reload before trying again.');
    }
    return data;
}

export function Connections({bootstrap, states = {}, onReconnect, onClose}: {
    bootstrap: Bootstrap; states?: Record<string, Status>;
    onReconnect?: () => void; onClose: () => void;
}) {
    const initialMethod = bootstrap.connectionForm === 'import' ? 'credential' : 'invitation';
    const initialScreen = bootstrap.connectionError && ['pair', 'import'].includes(bootstrap.connectionForm || '') ? 'add' : 'list';
    const [data, setData] = useState<Bootstrap>(bootstrap);
    const [screen, setScreen] = useState<Screen>(initialScreen);
    const [selectedId, setSelectedId] = useState<string | null>(null);
    const [method, setMethod] = useState<Method>(initialMethod);
    const [name, setName] = useState('');
    const [invitation, setInvitation] = useState('');
    const [credential, setCredential] = useState('');
    const [notice, setNotice] = useState(bootstrap.connectionError || bootstrap.connectionStatus || '');
    const [error, setError] = useState(Boolean(bootstrap.connectionError));
    const [busy, setBusy] = useState(false);
    const [uncertain, setUncertain] = useState(false);
    const [confirmRemove, setConfirmRemove] = useState(false);
    const dialog = useRef<HTMLDialogElement>(null);
    const heading = useRef<HTMLHeadingElement>(null);
    const nameField = useRef<HTMLInputElement>(null);
    const selected = data.vessels.find(vessel => vessel.id === selectedId);
    const count = data.vessels.length;
    const online = data.vessels.filter(vessel => states[vessel.id]?.connected).length;
    const secret = method === 'invitation' ? invitation : credential;

    useEffect(() => { dialog.current?.showModal(); return () => dialog.current?.close(); }, []);
    useEffect(() => {
        if (screen === 'add') nameField.current?.focus();
        else heading.current?.focus();
    }, [screen]);

    function back() {
        setConfirmRemove(false);
        setScreen(screen === 'guide' ? 'add' : 'list');
    }
    function close() { if (!busy) onClose(); }
    function openAdd() {
        setNotice(''); setError(false); setUncertain(false);
        setScreen('add');
    }
    function onCancel(event: React.SyntheticEvent) {
        event.preventDefault();
        if (busy) return;
        if (confirmRemove) setConfirmRemove(false);
        else if (screen !== 'list') back();
        else onClose();
    }
    async function send(path: string, body: unknown, method: 'POST' | 'DELETE' = 'POST') {
        if (busy || uncertain) return;
        setBusy(true); setNotice(''); setError(false);
        try {
            const response = await fetch(path, {
                method, credentials: 'same-origin',
                headers: {'X-Helm-Client': 'react', 'Content-Type': 'application/json', Accept: 'application/json',
                    'X-CSRF-TOKEN': document.querySelector<HTMLMetaElement>('meta[name="csrf-token"]')?.content || ''},
                body: JSON.stringify(body),
            });
            if (!response.redirected) throw new Error('Connection outcome uncertain. Check status before trying again.');
            const updated = bootstrapFrom(await response.text());
            setData(updated);
            if (updated.connectionError) {
                setNotice(updated.connectionError); setError(true);
                return; // Keep both private drafts in memory for review; do not replay.
            }
            location.assign('/?manage-vessels=1');
        } catch {
            setUncertain(true); setError(true);
            setNotice('Connection outcome uncertain. Check status before trying again; your entries are retained in this window.');
        } finally { setBusy(false); }
    }
    async function checkStatus() {
        if (busy) return;
        setBusy(true);
        try {
            const response = await fetch('/?manage-vessels=1', {credentials: 'same-origin', headers: {Accept: 'text/html'}});
            if (!response.ok || response.redirected) throw new Error('Status unavailable');
            const updated = bootstrapFrom(await response.text());
            setData(updated); setUncertain(false);
            const found = updated.vessels.some(vessel => vessel.name === name);
            const pending = updated.pairings?.some(pairing => pairing.name === name);
            if (screen === 'add' && (found || pending)) {
                setScreen('list'); setNotice(found ? 'A saved connection has this name. Review its details before another attempt.' : 'A pairing has this name. Check it before another attempt.');
                setError(false);
                return;
            }
            setNotice('No matching connection was found. Review the fields and pending pairings before another attempt.');
            setError(false);
        } catch {
            setNotice('Status could not be checked. Keep this window open and try Check status again.');
            setError(true);
        } finally { setBusy(false); }
    }
    function viewDetails(id: string) { setSelectedId(id); setConfirmRemove(false); setScreen('details'); }
    const statusLabel = (id: string) => {
        const status = states[id];
        return !status ? 'Saved' : status.connected ? 'Connected' : status.status === 'Connecting…' ? 'Connecting' : 'Unavailable';
    };
    const title = screen === 'list' ? 'Your Vessels' : screen === 'add' ? 'Add a Vessel' : screen === 'guide' ? 'Get an invitation' : selected?.name || 'Vessel details';
    const command = `STATE="/path/to/vessel/state"\nENDPOINT="https://vessel.example.com"\nINVITE_DIR=$(mktemp -d)\nvessel pair-invite --directory "$STATE" \\\n  --endpoint "$ENDPOINT" \\\n  --principal ${data.principalId || 'YOUR_PRINCIPAL_ID'} \\\n  --full-access \\\n  --output "$INVITE_DIR/invitation.json" &&\ncat "$INVITE_DIR/invitation.json"`;

    return <dialog className="settings-dialog connections-dialog" ref={dialog} onCancel={onCancel} aria-labelledby="vessel-manager-title">
        <header className="connections-header">
            {screen !== 'list' && <button type="button" className="connections-back" disabled={busy} onClick={back} aria-label="Back">←</button>}
            <div className="connections-title"><h2 id="vessel-manager-title" ref={heading} tabIndex={-1}>{title}</h2>
                <p>{screen === 'list' ? `${count} saved · ${online} connected` : screen === 'add' ? 'Connect a computer where your voyages will run.' : screen === 'guide' ? 'Create a private invitation on the Vessel host.' : 'Connection details and access'}</p></div>
            {screen === 'list' && <button type="button" className="connections-add" onClick={openAdd} disabled={busy || count >= 64}>+ Add Vessel</button>}
            <button type="button" className="connections-close" aria-label="Close Vessel connections" disabled={busy} onClick={close}>×</button>
        </header>
        {notice && <div className={`connections-notice ${error ? 'is-error' : ''}`} role={error ? 'alert' : 'status'}>{notice}
            {uncertain && <button type="button" onClick={()=>void checkStatus()} disabled={busy}>Check status</button>}</div>}
        <div className="connections-body">
            {screen === 'list' && <>
                <p className="connections-intro">These computers run your voyages. Provider credentials stay on each Vessel.</p>
                {(data.pairings || []).length > 0 && <section className="connections-pending" aria-label="Pending pairings"><h3>Needs confirmation</h3>
                    {(data.pairings || []).map(pairing => <div key={pairing.id} className="connections-pending-row"><div><strong>{pairing.name}</strong><small>The original pairing is still pending. Check it before adding another.</small></div><button type="button" disabled={busy || uncertain} onClick={()=>void send(`/connections/pair/${encodeURIComponent(pairing.id)}/retry`, {})}>Check connection</button></div>)}
                </section>}
                {count ? <div className="connections-grid">{data.vessels.map(vessel => <article className="connections-card" key={vessel.id}>
                    <div className="connections-card-top"><span className="connections-server" aria-hidden="true">▣</span><span className={`connections-status ${states[vessel.id]?.connected ? 'is-connected' : ''}`}><i aria-hidden="true"/>{statusLabel(vessel.id)}</span></div>
                    <h3>{vessel.name}</h3><p>{endpointHost(vessel.endpoint)}</p>
                    <button type="button" className="connections-card-action" onClick={()=>viewDetails(vessel.id)}>View details <span aria-hidden="true">→</span></button>
                </article>)}</div> : <div className="connections-empty"><span aria-hidden="true">▣</span><h3>No Vessels yet</h3><p>Add a computer you control to start a voyage from Helm Web.</p><button type="button" onClick={openAdd}>Add your first Vessel</button></div>}
                {count >= 64 && <p>Connection limit reached. Remove a Vessel before adding another.</p>}
            </>}
            {screen === 'add' && <>
                <div className="connections-method" aria-label="Connection method"><button type="button" aria-pressed={method === 'invitation'} disabled={busy} onClick={()=>setMethod('invitation')}>New invitation</button><button type="button" aria-pressed={method === 'credential'} disabled={busy} onClick={()=>setMethod('credential')}>Existing credential</button></div>
                <form id="vessel-add-form" onSubmit={event=>{event.preventDefault();void send(method === 'invitation' ? '/connections/pair' : '/connections', {name, [method]: secret});}}>
                    <label>Name<input ref={nameField} required maxLength={100} value={name} onChange={event=>setName(event.target.value)} placeholder="e.g. My workstation" disabled={busy} autoComplete="off"/></label>
                    <label>{method === 'invitation' ? 'Invitation JSON' : 'Connection credential JSON'}<textarea required maxLength={16384} rows={5} value={secret} onChange={event=>method === 'invitation' ? setInvitation(event.target.value) : setCredential(event.target.value)} placeholder={method === 'invitation' ? 'Paste the private invitation from your Vessel' : 'Paste an existing Vessel connection credential'} disabled={busy} autoComplete="off" spellCheck={false}/></label>
                    <p className="connections-private">This private value goes to Helm Web for pairing and is never placed in a voyage message. {method === 'invitation' ? 'Invitations expire after 10 minutes.' : 'Importing the same Vessel replaces this tenant’s saved connection.'}</p>
                    {method === 'invitation' && <button type="button" className="connections-help-link" onClick={()=>setScreen('guide')}>How do I get an invitation? →</button>}
                </form>
            </>}
            {screen === 'guide' && <div className="connections-guide">
                <ol><li>Install Vessel and Voyage on a Linux machine you control.</li><li>Expose its authenticated public HTTPS/WSS endpoint. A local service alone is not reachable from Helm Web.</li><li>On that machine, replace the paths and endpoint below, then run this command. Paste the resulting invitation into Add a Vessel.</li></ol>
                <pre>{command}</pre><p>Keep the invitation private. It grants this Web account full access to that Vessel and expires after 10 minutes.</p>
                <a href={guideUrl} target="_blank" rel="noopener noreferrer">Full first-time setup guide ↗</a>
            </div>}
            {screen === 'details' && selected && <div className="connections-details"><div className="connections-detail-card"><span className="connections-status"><i aria-hidden="true"/>{statusLabel(selected.id)}</span><dl><div><dt>Address</dt><dd>{selected.endpoint || 'Not available'}</dd></div><div><dt>Vessel ID</dt><dd>{selected.vessel_id}</dd></div></dl></div>
                {!states[selected.id]?.connected && onReconnect && <button type="button" onClick={onReconnect}>Reconnect Vessels</button>}
                <section className="connections-danger"><h3>Remove from Helm Web</h3><p>Removing this connection stops new browser access through this Web account. It does not stop voyages or revoke the underlying Vessel grant used by other clients.</p>
                    {!confirmRemove ? <button type="button" onClick={()=>setConfirmRemove(true)}>Remove connection…</button> : <div className="connections-confirm" role="group" aria-label={`Confirm removing ${selected.name}`}><p>Remove <strong>{selected.name}</strong> from this Web account?</p><button type="button" disabled={busy} onClick={()=>setConfirmRemove(false)}>Keep connection</button><button type="button" className="connections-remove" disabled={busy || uncertain} onClick={()=>void send(`/connections/${encodeURIComponent(selected.id)}`, {confirm_disconnect: 1}, 'DELETE')}>Remove {selected.name}</button></div>}
                </section>
            </div>}
        </div>
        <footer className="connections-footer">
            {screen === 'list' ? <button type="button" onClick={close}>Done</button> : screen === 'add' ? <><button type="button" onClick={back} disabled={busy}>Back</button><button className="connections-primary" type="submit" form="vessel-add-form" disabled={busy || uncertain || !name.trim() || !secret.trim()}>{busy ? 'Connecting…' : 'Connect Vessel'}</button></> : <button type="button" onClick={back} disabled={busy}>{screen === 'guide' ? 'Back to adding' : 'Back to Vessels'}</button>}
        </footer>
    </dialog>;
}
