import {HostBrowser} from './HostBrowser';
import React, {useEffect, useRef, useState, useSyncExternalStore} from 'react';
import {ToolGroup,threadRows} from './ToolGroup';
import {Connections} from './Connections';
import {ImagePart,Output} from './MessageParts';
import {VoyageActions} from './VoyageActions';
import {Settings} from './Settings';
import {Decisions} from './Decisions';
import {VesselFleet} from '../js/vessel-fleet.js';
import {voyageList, activityLabel, cardStatus} from './presentation';
import {marked} from 'marked';
import DOMPurify from 'dompurify';
import {Workspace, type Tab} from './workspace';

type Bootstrap = {tenantId: string; vessels: any[]; ticketUrl: string; legacyUrl: string; connectionsUrl: string; logoutUrl: string};
const csrf = () => document.querySelector<HTMLMetaElement>('meta[name="csrf-token"]')?.content || '';
function Icon({name}: {name: string}) {
    const paths: Record<string, string> = {plus:'M12 5v14M5 12h14',search:'m21 21-5-5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0',folder:'M3 7V5h6l2 2h10v13H3Z',clip:'m9 17 8-8a3 3 0 0 0-4-4l-9 9a5 5 0 0 0 7 7l9-9',up:'m6 12 6-6 6 6M12 6v14',stop:'M6 6h12v12H6Z',menu:'M4 8h16M4 16h16',refresh:'M20 7v5h-5M4 17v-5h5M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1',server:'M3 4h18v6H3ZM3 14h18v6H3ZM6 7h1M6 17h1',chevron:'m8 10 4 4 4-4',user:'M8 7a4 4 0 1 0 8 0 4 4 0 0 0-8 0M4 21v-2a8 8 0 0 1 16 0v2'};
    return <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={paths[name] || paths.chevron}/></svg>;
}
function content(value: unknown): string {
    if (typeof value === 'string') return value;
    if (Array.isArray(value)) return value.map(part => part?.text || (part?.type === 'image' ? '[Image attachment]' : JSON.stringify(part))).join('\n');
    return value == null ? '' : JSON.stringify(value, null, 2);
}
export function prose(text: string) {
    return DOMPurify.sanitize(marked.parse(text.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/g,'')) as string, {ALLOWED_TAGS:['p','br','strong','em','del','code','pre','blockquote','ul','ol','li','h1','h2','h3','h4','hr','a','table','thead','tbody','tr','th','td'], ALLOWED_ATTR:['href','title'], ALLOW_DATA_ATTR:false});
}
function Composer({tab, workspace, legacyUrl,onSettings}: {tab?: Tab; workspace: Workspace; legacyUrl: string; onSettings:()=>void}) {
    const fileInput = useRef<HTMLInputElement>(null);
    const running = ['running','starting','cancelling'].includes(tab?.snapshot?.run?.state);
    const enabled = tab && workspace.actionable(tab) && workspace.permitted(tab,running?'steer':'submit');
    const send = () => { if (tab && enabled) void workspace.act(tab.key, running ? 'steer' : 'submit'); };
    return <form className="composer" aria-label="Message composer" onPaste={event=>{if(tab&&event.clipboardData.files.length){event.preventDefault();void workspace.attach(tab.key,[...event.clipboardData.files]);}}} onDragOver={event=>{if(event.dataTransfer.types.includes('Files'))event.preventDefault();}} onDrop={event=>{if(tab&&event.dataTransfer.files.length){event.preventDefault();void workspace.attach(tab.key,[...event.dataTransfer.files]);}}} onSubmit={event => {event.preventDefault(); send();}}>
        <div className="composer-box"><input ref={fileInput} type="file" accept="image/png,image/jpeg,image/webp" multiple hidden onChange={event=>{if(tab)void workspace.attach(tab.key,[...(event.target.files||[])]);event.target.value='';}}/>{!!tab?.pictures.length&&<div className="pictures">{tab.pictures.map(picture=><figure key={picture.id}><img src={picture.url} alt={picture.name}/><button type="button" disabled={tab.busy} aria-label={`Remove ${picture.name}`} onClick={()=>workspace.removePicture(tab.key,picture.id)}>×</button></figure>)}</div>}<textarea aria-label="Message" rows={2} value={tab?.draft || ''} disabled={!tab} onChange={event => tab && workspace.draft(tab.key,event.target.value)} placeholder="Ask anything…" onKeyDown={event => {if(event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing){event.preventDefault();send();}}}/>
        <div className="composer-toolbar"><div className="composer-options">
            <button type="button" onClick={onSettings} title="Location"><Icon name="folder"/><span>{tab?.snapshot?.workspace || 'Location'}</span><Icon name="chevron"/></button>
            <button type="button" disabled={!tab||tab.busy} aria-label="Attach pictures" title="Attach pictures" onClick={()=>fileInput.current?.click()}><Icon name="clip"/></button>
            <button type="button" onClick={onSettings} title="Account and model"><span>{tab?.snapshot?.inference?.model || 'Account & model'}</span><Icon name="chevron"/></button>
            <button type="button" onClick={onSettings} title="Provider account"><Icon name="user"/><span>Account</span><Icon name="chevron"/></button><button type="button" onClick={onSettings} title="Service tier"><span>{tab?.snapshot?.inference?.service_tier||'Service'}</span><Icon name="chevron"/></button><button type="button" onClick={onSettings} title="Reasoning"><span>{tab?.snapshot?.inference?.reasoning_effort || 'Reasoning'}</span><Icon name="chevron"/></button>
            <div className="access-picker"><label className="sr-only" htmlFor={`access-${tab?.session||'new'}`}>Voyage access mode</label><select id={`access-${tab?.session||'new'}`} disabled={!enabled} value={tab?.snapshot?.access||''} title="Access is enforced by the executing host; configured roots and limits still apply." onChange={event=>{if(tab)void workspace.act(tab.key,'set_access',{access:event.target.value});}}><option value="" disabled>Access unknown</option><option value="read-only">Read only</option><option value="approval">Approval</option><option value="unrestricted">Full access</option></select></div>
        </div>{running && <button type="button" className="icon-button" aria-label="Cancel run" disabled={!enabled} onClick={() => tab && void workspace.act(tab.key,'cancel')}><Icon name="stop"/></button>}<button className="send-button icon-button" aria-label={running ? 'Steer' : 'Send'} disabled={!enabled || (!tab?.draft.trim() && !tab?.pictures.length)}><Icon name="up"/></button></div></div>
        <div className="preview-caption"><span>React preview</span><a href={legacyUrl}>Compare with existing console ↗</a></div>
    </form>;
}
export function Conversation({tab, workspace, active, legacyUrl,onSettings}: {tab: Tab; workspace: Workspace; active: boolean; legacyUrl: string; onSettings:()=>void}) {
    const run = tab.snapshot?.run, scroll = useRef<HTMLDivElement>(null), following = useRef(true);
    const [showJump,setShowJump] = useState(false);
    useEffect(() => {if(active && following.current && scroll.current) scroll.current.scrollTop=scroll.current.scrollHeight;},[active,tab.snapshot]);
    const renderMessage=(message:any)=>{return <article key={message.message_index} className={`message ${message.role}`}>
                    {['tool','function'].includes(message.role) ? <pre>{content(message.content)}</pre> : <><span className="sr-only">{message.role}</span><div className="prose" dangerouslySetInnerHTML={{__html:prose(message.parts?.length?message.parts.filter((part:any)=>part.type==='text').map((part:any)=>part.text).join('\n'):content(message.content))}}/></>}
                    {message.interrupted_attempt&&<small className="message-meta">Interrupted attempt</small>}
                    {message.parts?.filter((part:any)=>part.type==='image').map((part:any,index:number)=><ImagePart key={part.attachment?.id||index} attachment={part.attachment} tab={tab} workspace={workspace}/>)}
                    {message.projection_truncated && <button disabled={tab.busy} onClick={()=>void workspace.expand(tab.key,message.message_index)}>Read complete message</button>}
                </article>;};
    return <section className="conversation" hidden={!active} aria-label={tab.title}>
        <h1 className="sr-only">{tab.title}</h1>
        {tab.notice && <aside className="notice" role="status">{tab.notice}<button onClick={() => void workspace.reconcile(tab.key)}>Check receipts</button></aside>}
        <div className="transcript" ref={scroll} tabIndex={0} aria-label="Conversation messages" onScroll={() => {const el=scroll.current!;following.current=el.scrollHeight-el.scrollTop-el.clientHeight<80;setShowJump(!following.current);}}><div className="thread">
            {!tab.snapshot && <p className="empty">Waiting for a current Vessel snapshot…</p>}
            {tab.snapshot?.message_offset > 0 && <button className="history-link" disabled={tab.busy} onClick={async()=>{const el=scroll.current!;const height=el.scrollHeight;following.current=false;await workspace.earlier(tab.key);requestAnimationFrame(()=>{el.scrollTop+=el.scrollHeight-height;});}}>Load earlier messages</button>}
            {threadRows(tab.snapshot?.messages||[]).map(row=>row.entries?<ToolGroup key={row.key} entries={row.entries} running={['running','starting','cancelling'].includes(run?.state)} messageStart={run?.message_start} decisions={tab.decisions.length>0} renderMessage={renderMessage}/>:<React.Fragment key={row.key}>{renderMessage(row.message)}</React.Fragment>)}
            <Output tab={tab} workspace={workspace}/>
            {(run?.tool_previews || []).filter((preview:any)=>!(tab.snapshot?.messages||[]).some((message:any)=>message.tool_calls?.some((call:any)=>call.id===preview.call_id))).map((preview:any,index:number) => <details className="tool-entry" key={index}><summary>Tool preview · {preview.name || 'Tool'}</summary><pre>{content(preview.arguments)}</pre></details>)}
            {(run?.reasoning_previews||[]).map((preview:any,index:number)=><details className="tool-entry" key={index}><summary>{preview.kind==='summary'?'Reasoning summary':'Provider thinking'} · {preview.finalized?'finalized disclosure':'streaming · provisional'}</summary><pre>{preview.text}</pre>{preview.truncated&&<small>Preview truncated</small>}</details>)}
        </div></div>
        {showJump && <button className="jump" onClick={() => {following.current=true;setShowJump(false);scroll.current?.scrollTo({top:scroll.current.scrollHeight});}}>Jump to latest ↓</button>}
        <Decisions tab={tab} workspace={workspace}/>
        <Composer tab={tab} workspace={workspace} legacyUrl={legacyUrl} onSettings={onSettings}/>
    </section>;
}

export function App({bootstrap}: {bootstrap: Bootstrap}) {
    const [runtime] = useState(() => {
        let workspace: Workspace;
        const fleet = new VesselFleet(bootstrap.vessels, {tenantId: bootstrap.tenantId, changed: () => workspace.connectionChanged(), ticket: async (vessel: string) => {
            const response = await fetch(bootstrap.ticketUrl, {method: 'POST', credentials: 'same-origin', headers: {'Content-Type': 'application/json', Accept: 'application/json', 'X-CSRF-TOKEN': csrf()}, body: JSON.stringify({vessel})});
            if (!response.ok) throw Object.assign(new Error('Vessel authorization unavailable'), {permanent: [401, 403, 404, 419].includes(response.status)});
            return response.json();
        }});
        workspace = new Workspace(() => fleet.connections);
        return {fleet, workspace};
    });
    const {fleet, workspace} = runtime;
    useSyncExternalStore(workspace.subscribe, workspace.getVersion, workspace.getVersion);
    const [active, setActive] = useState<string | null>(null), [query, setQuery] = useState('');
    useEffect(() => {
        fleet.start(); const timer = setInterval(() => { fleet.poll(); for (const key of workspace.tabs.keys()) void workspace.refresh(key); workspace.changed(); }, 5000);
        return () => { clearInterval(timer); workspace.close(); fleet.close(); };
    }, [runtime]);
    const [manage,setManage] = useState(()=>typeof location!=='undefined'&&new URLSearchParams(location.search).has('manage-vessels'));
    const [settings,setSettings] = useState<{tab?:Tab}|null>(null);
    const [mobile,setMobile] = useState(false);
    const [appearance,setAppearance] = useState(() => {try{return localStorage.getItem('flux.appearance') || 'system';}catch{return 'system';}});
    useEffect(() => {
        const media=window.matchMedia('(prefers-color-scheme: dark)');
        const update=()=>document.documentElement.classList.toggle('dark',appearance==='dark'||(appearance==='system'&&media.matches));
        update();media.addEventListener('change',update);try{localStorage.setItem('flux.appearance',appearance);}catch{}
        return ()=>media.removeEventListener('change',update);
    },[appearance]);
    const connections=[...fleet.connections.values()], voyages=voyageList(connections,query,(connection,voyage)=>{const tab=workspace.tabs.get(JSON.stringify([connection.id,voyage.session_id]));return tab&&!tab.stale&&Date.now()-tab.freshAt<35000?tab.snapshot:null;});
    const selected=active ? workspace.tabs.get(active) : null;
    return <div className="helm-preview">
        <button className="mobile-toggle icon-button" aria-label="Open voyage navigation" aria-expanded={mobile} onClick={()=>setMobile(!mobile)}><Icon name="menu"/></button>
        {mobile && <button className="sidebar-backdrop" aria-label="Close voyage navigation" onClick={()=>setMobile(false)}/>}
        <aside className={`sidebar ${mobile?'mobile-open':''}`} aria-label="Voyages">
            <header className="sidebar-heading"><h2>Voyages <span>{voyages.length || ''}</span></h2><button className="new-voyage icon-button" onClick={()=>setSettings({})} aria-label="New voyage" title="New voyage"><Icon name="plus"/></button><button className="mobile-close icon-button" aria-label="Close voyage navigation" onClick={()=>setMobile(false)}>×</button></header>
            <div className="search"><Icon name="search"/><input aria-label="Find a voyage or Vessel" type="search" value={query} onChange={event=>setQuery(event.target.value)} placeholder="Search voyages…"/></div>
            <nav className="voyage-list" aria-label="Voyages">{voyages.map((voyage:any)=>{
                const key=JSON.stringify([voyage.connection.id,voyage.session_id]),tab=workspace.tabs.get(key);
                const status=cardStatus(voyage,Boolean(voyage.connection.client),tab&&!tab.stale ? tab.snapshot : null);
                return <div className="voyage-row" key={key}><button className="voyage-card" data-status-tone={status.tone} data-animated={status.animated || undefined} aria-current={active===key} onClick={()=>{setActive(workspace.open(voyage.connection.id,voyage.session_id,voyage.name||voyage.session_id));setMobile(false);}}>
                    <span className="card-title">{voyage.name||voyage.session_id}{tab?.draft && <span title="Unsent draft"> •</span>}</span><span className="card-meta"><span>{voyage.connection.name}</span><time title={voyage.activity?.iso}>{activityLabel(voyage.activity)}</time></span><span className="card-status"><i aria-hidden="true"/>{status.label}</span>
                </button><VoyageActions connection={voyage.connection} voyage={voyage} onChanged={()=>workspace.connectionChanged()}/></div>;
            })}{!voyages.length && <p className="empty">{query?'No matching voyages.':'No voyages yet.'}</p>}</nav>
            <footer className="sidebar-footer"><p className="connection-state" role="status">{selected ? selected.stale?'Reconnecting…':`Connected · ${selected.snapshot?.run?.state||'idle'}` : connections.some((c:any)=>c.client)?'Ready':connections.length?'Connecting…':'No Vessels connected'}</p>
                <div className="footer-controls"><details className="popover connections"><summary><Icon name="server"/><span>{connections.filter((c:any)=>c.client).length}/{connections.length} connected</span><Icon name="chevron"/></summary><div className="popover-panel"><strong>Vessel connections</strong>{connections.map((c:any)=><p key={c.id}>{c.name} · {c.status}</p>)}<button onClick={()=>setManage(true)}>Manage Vessels</button></div></details>
                <button className="icon-button" aria-label="Reconnect Vessels" title="Reconnect Vessels" onClick={()=>fleet.reconnect()}><Icon name="refresh"/></button>
                <details className="popover profile"><summary aria-label="Profile menu"><Icon name="user"/></summary><div className="popover-panel"><strong>Appearance</strong><div className="appearance">{['light','dark','system'].map(mode=><button key={mode} aria-pressed={appearance===mode} onClick={()=>setAppearance(mode)}>{mode}</button>)}</div><button onClick={()=>setManage(true)}>Vessel connections</button><a href={bootstrap.legacyUrl}>Existing console ↗</a><form action={bootstrap.logoutUrl} method="post"><input type="hidden" name="_token" value={csrf()}/><button>Sign out</button></form></div></details></div>
                <a className="preview-label" href={bootstrap.legacyUrl} title="Return to the existing console">React preview · existing console ↗</a>
            </footer>
        </aside>
        <main aria-label="Conversation">{!active && <section className="conversation"><div className="transcript"><div className="thread empty">Choose a voyage from any connected Vessel. <button onClick={()=>setSettings({})}>New voyage</button></div></div><Composer workspace={workspace} legacyUrl={bootstrap.legacyUrl} onSettings={()=>setSettings({})}/></section>}
            {selected && <HostBrowser key={selected.key} tab={selected} client={fleet.connections.get(selected.vessel)?.client}/> }
            {[...workspace.tabs.values()].map(tab=><Conversation key={tab.key} tab={tab} workspace={workspace} active={active===tab.key} legacyUrl={bootstrap.legacyUrl} onSettings={()=>setSettings({tab})}/>)}
        </main>
        {manage&&<Connections bootstrap={bootstrap} onClose={()=>setManage(false)}/>}
        {settings&&<Settings fleet={fleet} workspace={workspace} tab={settings.tab} tenant={bootstrap.tenantId} onClose={()=>setSettings(null)} onCreated={(vessel,process)=>setActive(workspace.open(vessel,process.session_id,process.name||'New voyage'))}/>}
    </div>;
}
