import React,{useEffect,useRef} from 'react';
import {sidebarActions} from '../js/sidebar-actions.js';
export function VoyageActions({connection,voyage,onChanged}:{connection:any;voyage:any;onChanged:()=>void}){
    const host=useRef<HTMLDivElement>(null),state=useRef({connection,voyage,onChanged});state.current={connection,voyage,onChanged};
    useEffect(()=>{
        const root=host.current!;
        root.innerHTML=`<details class="popover voyage-menu"><summary aria-label="Voyage actions">⋯</summary><div class="popover-panel" data-actions></div></details><dialog class="settings-dialog" data-dialog><form id="sidebar-action-form"><header><h2 id="sidebar-action-title"></h2></header><p id="sidebar-action-target"></p><p id="sidebar-action-status" role="status"></p><pre id="sidebar-action-details" hidden></pre><label id="sidebar-name-field" hidden>Name<input id="sidebar-name" maxlength="256"></label><label id="sidebar-access-field" hidden>Access mode<select id="sidebar-access"><option value="read-only">Read only</option><option value="approval">Approval</option><option value="unrestricted">Full access</option></select></label><label id="sidebar-branch-field" hidden>Branch through<select id="sidebar-branch"></select><button id="sidebar-branch-more" type="button">Load more branch points</button></label><label id="sidebar-retain-field" hidden>Recent messages to retain<input id="sidebar-retain" type="number" min="0" max="4294967295" value="128"></label><label id="sidebar-confirm-field" hidden><span id="sidebar-confirm-label"></span><input id="sidebar-confirm" autocomplete="off"></label><footer><button id="sidebar-reconcile" type="button">Check pending receipt</button><button id="sidebar-dismiss" type="button">Close</button><button id="sidebar-submit" type="submit" disabled>Confirm</button></footer></form></dialog>`;
        const dialog=root.querySelector<HTMLDialogElement>('[data-dialog]')!;
        const adapter=sidebarActions(root,{changed:()=>state.current.onChanged(),modal:()=>({show:()=>dialog.showModal(),close:()=>dialog.close()})});
        dialog.addEventListener('cancel',()=>adapter.invalidate());
        const panel=root.querySelector('[data-actions]')!;
        for(const [action,label] of Object.entries({rename:'Rename',access:'Access mode',archive:'Archive / Restore',branch:'Branch',cancel:'Cancel run',compact:'Compact context',clear:'Clear conversation',delete:'Delete',details:'Details'})){
            const button=document.createElement('button');button.type='button';button.textContent=label;button.onclick=()=>{root.querySelector('details')!.open=false;void adapter.open(state.current.connection,state.current.voyage,action);};panel.append(button);
        }
        return()=>{adapter.invalidate();dialog.close();root.replaceChildren();};
    },[connection.id,voyage.session_id]);
    return <div ref={host} className="voyage-action-host"/>;
}
