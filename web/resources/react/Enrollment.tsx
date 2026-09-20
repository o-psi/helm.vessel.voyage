import React,{useEffect,useRef} from 'react';
import {accountEnrollment} from '../js/account-enrollment.js';
// This isolated island retains the audited private enrollment/receipt lifecycle.
// React owns only its host; the adapter owns all descendants until disposal.
export function Enrollment({connection,workspace,tenant,onRefreshed}:{connection:any;workspace:string;tenant:string;onRefreshed:()=>void}){
    const host=useRef<HTMLDivElement>(null),current=useRef({connection,workspace,onRefreshed});current.current={connection,workspace,onRefreshed};
    useEffect(()=>{
        const root=host.current!;root.innerHTML=`<button type="button" data-open>Add ChatGPT account</button><section id="enrollment-panel" hidden aria-label="Add ChatGPT account"><header><strong id="enrollment-title"></strong><button id="enrollment-close" type="button" aria-label="Close account sign-in">×</button></header><p>Your sign-in stays on your Vessel.</p><div id="enrollment-setup"><div id="enrollment-provider-field" hidden><label>ChatGPT connection<select id="enrollment-provider"></select></label></div><label>Account name<input id="enrollment-label" maxlength="128" autocomplete="off" placeholder="Personal or Work"></label></div><div id="enrollment-private" hidden><p>Enter this code on the ChatGPT sign-in page.</p><p id="enrollment-code" aria-label="Private sign-in code"></p><a id="enrollment-link" target="_blank" rel="noopener noreferrer">Open ChatGPT sign-in ↗</a></div><p id="enrollment-status" role="status"></p><button id="enrollment-cancel" type="button" hidden>Cancel sign-in</button><button id="enrollment-check" type="button" hidden>Check sign-in</button><button id="enrollment-start" type="button" disabled>Continue with ChatGPT</button></section>`;
        const adapter=accountEnrollment(root,{context:()=>({connection:current.current.connection,workspace:current.current.workspace}),refreshed:()=>current.current.onRefreshed()});
        root.querySelector('[data-open]')!.addEventListener('click',()=>void adapter.open());
        return()=>{adapter.dispose();root.replaceChildren();};
    },[connection.id,workspace]);
    return <div ref={host} data-tenant-id={tenant} className="enrollment"/>;
}
