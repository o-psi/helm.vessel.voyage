import test from 'node:test';
import assert from 'node:assert/strict';
import React from 'react';
import {act} from 'react';
import {createRoot} from 'react-dom/client';
import {JSDOM} from 'jsdom';
import {voyageLocation,voyagePath} from '../resources/react/voyage-url.ts';
import {App} from '../resources/react/App.tsx';
import {VesselFleet} from '../resources/js/vessel-fleet.js';
import {Workspace} from '../resources/react/workspace.ts';
const a='11111111-2222-4333-8444-555555555555', b='22222222-3333-4444-8555-666666666666';
const s='33333333-4444-4555-8666-777777777777', t='44444444-5555-4666-8777-888888888888';
test('location identifies both Vessel and voyage and rejects unrelated paths',()=>{
    assert.deepEqual(voyageLocation(voyagePath(a,s)),{vessel:a,session:s});
    assert.notEqual(voyagePath(a,s),voyagePath(b,s));
    for(const path of ['/', '/voyage', '/voyages/'+a+'/wrong', '/voyages/'+a+'/'+s+'/extra', '/voyages/%2F/'+s]) assert.equal(voyageLocation(path),null);
});
test('direct link restores after catalogue, selection changes URL, history restores and missing voyage stays unselected',async()=>{
    const dom=new JSDOM('<meta name="csrf-token" content="fixture"><div id="mount"></div>',{url:`https://helm.test${voyagePath(b,s)}`});
    const saved={window:globalThis.window,document:globalThis.document,location:globalThis.location,IS_REACT_ACT_ENVIRONMENT:globalThis.IS_REACT_ACT_ENVIRONMENT};
    Object.assign(globalThis,{window:dom.window,document:dom.window.document,location:dom.window.location,IS_REACT_ACT_ENVIRONMENT:true});
    dom.window.HTMLDialogElement.prototype.close=function(){this.open=false;};
    Object.defineProperty(dom.window,'matchMedia',{value:()=>({matches:false,addEventListener(){},removeEventListener(){}})});
    const start=VesselFleet.prototype.start,close=VesselFleet.prototype.close,poll=VesselFleet.prototype.poll,refresh=Workspace.prototype.refresh;
    let fleet: VesselFleet;
    VesselFleet.prototype.start=function(){fleet=this;}; VesselFleet.prototype.close=function(){};VesselFleet.prototype.poll=function(){};
    Workspace.prototype.refresh=async function(){};
    const root=createRoot(dom.window.document.getElementById('mount')!);
    let reload: ReturnType<typeof createRoot> | null=null;
    const bootstrap={tenantId:'tenant',vessels:[{id:a,name:'First',vessel_id:a},{id:b,name:'Second',vessel_id:b}],ticketUrl:'/console/ticket',legacyUrl:'/',connectionsUrl:'/connections',logoutUrl:'/console/logout'};
    try {
        await act(async()=>root.render(React.createElement(App,{bootstrap})));
        assert.match(dom.window.document.body.textContent!,/Waiting for this voyage/);
        await act(async()=>{fleet!.connections.get(a).voyages=[{session_id:s,name:'Same session / first'},{session_id:t,name:'Other'}];fleet!.connections.get(a).voyages=[{session_id:s,name:'Same session / first'},{session_id:t,name:'Other'}];fleet!.connections.get(b).voyages=[{session_id:s,name:'Same session / second'}];fleet!.changed();});
        assert.equal(dom.window.document.querySelector('[aria-current="true"]')?.textContent?.includes('Same session / second'),true);
        await act(async()=>{root.unmount();});
        reload=createRoot(dom.window.document.getElementById('mount')!);
        await act(async()=>reload.render(React.createElement(App,{bootstrap})));
        await act(async()=>{fleet!.connections.get(a).voyages=[{session_id:s,name:'Same session / first'},{session_id:t,name:'Other'}];fleet!.connections.get(b).voyages=[{session_id:s,name:'Same session / second'}];fleet!.changed();});
        assert.equal(dom.window.document.querySelector('[aria-current="true"]')?.textContent?.includes('Same session / second'),true);
        await act(async()=>{[...dom.window.document.querySelectorAll<HTMLButtonElement>('.voyage-card')].find(el=>el.textContent?.includes('Other'))!.click();});
        assert.equal(dom.window.location.pathname,voyagePath(a,t));
        assert.equal(dom.window.document.querySelector('[aria-current="true"]')?.textContent?.includes('Other'),true);
        await act(async()=>{dom.window.history.replaceState(null,'',voyagePath(b,s));dom.window.dispatchEvent(new dom.window.PopStateEvent('popstate'));});
        assert.equal(dom.window.document.querySelector('[aria-current="true"]')?.textContent?.includes('Same session / second'),true);
        await act(async()=>{dom.window.history.replaceState(null,'',voyagePath(a,'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'));dom.window.dispatchEvent(new dom.window.PopStateEvent('popstate'));});
        assert.equal(dom.window.document.querySelector('[aria-current="true"]'),null);
        assert.match(dom.window.document.body.textContent!,/Waiting for this voyage/);
    } finally {
        await act(async()=>{(reload || root).unmount();});
        Object.assign(VesselFleet.prototype,{start,close,poll});Workspace.prototype.refresh=refresh;
        Object.assign(globalThis,saved);dom.window.close();
    }
});
