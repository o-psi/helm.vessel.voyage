import React, {useEffect, useRef} from 'react';
import {vesselUpdate} from '../js/vessel-update.js';

// The same receipt controller is used across the cutover. It owns this static
// subtree, including its polling lifetime; React owns mounting and disposal.
export function VesselUpdateMarkup() {
    return <>
        <button type="button" id="setup-update-open" hidden>Vessel updates</button>
        <h3 id="update-vessel-name">Vessel update</h3>
        <p id="update-current"/>
        <p id="update-status" role="status"/>
        <div id="update-source">
            <label>Update source<select id="update-channel" defaultValue="stable">
                <option value="stable">Latest stable release</option>
                <option value="nightly">Latest completed development build</option>
            </select></label>
            <p>Development builds contain recent GitHub changes and may have unfinished features. The Vessel uses its own download access.</p>
            <button type="button" id="update-check">Check and prepare update</button>
        </div>
        <div id="update-review" hidden>
            <strong id="update-version"/><p id="update-description"/><p id="update-services"/>
            <p>Install this exact build on this Vessel? Its connection will briefly restart. Accounts and existing voyages are retained.</p>
            <button type="button" id="update-approve">Update this Vessel</button>
            <button type="button" id="update-discard">Not now</button>
        </div>
        <button type="button" id="update-refresh" hidden>Check update status</button>
        <button type="button" id="update-continue" hidden>Continue setup</button>
    </>;
}

export function VesselUpdate({connection, caps, tenant, onResume}: {connection:any; caps:any; tenant:string; onResume:()=>void}) {
    const root=useRef<HTMLDivElement>(null), resume=useRef(onResume);
    resume.current=onResume;
    useEffect(()=>{
        const controller=vesselUpdate(root.current!, {show:()=>{},resume:()=>resume.current()});
        controller.bind(connection,caps);
        root.current!.querySelector<HTMLElement>('#setup-update-open')!.hidden=true;
        return ()=>controller.dispose();
    },[connection,connection?.client,caps,tenant]);
    return <details open={!caps.features?.includes('execution_profiles')}>
        <summary>Vessel updates</summary>
        <div ref={root} data-tenant-id={tenant}><VesselUpdateMarkup/></div>
    </details>;
}
