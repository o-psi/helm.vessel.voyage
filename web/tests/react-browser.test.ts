import test from 'node:test';
import assert from 'node:assert/strict';
import React, {act} from 'react';
import {createRoot} from 'react-dom/client';
import {JSDOM} from 'jsdom';
import {HostBrowser, browserActivity} from '../resources/react/HostBrowser.tsx';
import type {Tab} from '../resources/react/workspace.ts';

const tick = () => new Promise(resolve => setTimeout(resolve, 0));
test('selected React browser uses shared controls, live revision fences and detach-only cleanup', async () => {
    const dom = new JSDOM('<div id="app"></div>', {url:'https://console.test'});
    const names = ['window','document','IS_REACT_ACT_ENVIRONMENT','RTCPeerConnection'] as const;
    const saved = names.map(name => Object.getOwnPropertyDescriptor(globalThis,name));
    const peers: any[] = [];
    class Peer {
        iceGatheringState = 'complete'; connectionState = 'new'; localDescription: any; closed = false;
        constructor() { peers.push(this); }
        async setRemoteDescription() {} async createAnswer() { return {type:'answer',sdp:'fixture'}; }
        async setLocalDescription(value: any) { this.localDescription = value; }
        close() { this.closed = true; }
    }
    Object.assign(globalThis,{window:dom.window,document:dom.window.document,IS_REACT_ACT_ENVIRONMENT:true,RTCPeerConnection:Peer});
    const root = createRoot(dom.window.document.getElementById('app')!);
    const sent: any[] = [];
    const socket = (name: string) => ({exchange:async (payload: any) => {
        const command = payload.command;
        sent.push({name,...command});
        const operation = command.operation;
        const binding = {incarnation:command.incarnation,browser_id:'browser',attachment_id:'attachment',tab_id:'tab',document_epoch:1,viewport_epoch:1,controller_epoch:1,capture_epoch:1};
        const status = {available:true,running:operation.action!=='status',binding:operation.action==='status'?null:binding,mode:'human',controller:'attachment',tabs:['tab']};
        return {protocol:1,outcome_unknown:false,result:{session_id:command.session_id,incarnation:command.incarnation,result:{status,value:operation.signal?.type==='request_offer'?{type:'offer',sdp:'fixture'}:null}}};
    }});
    let tab = {key:'one',vessel:'vessel-one',session:'session-one',title:'Selected task',incarnation:'owner-one',stale:false,snapshot:{revision:7}} as Tab;
    let client: any = socket('first');
    const render = () => act(async () => {root.render(React.createElement(HostBrowser,{key:tab.key,tab,client})); await tick();});
    const click = async (label: string) => act(async () => {
        const button = [...dom.window.document.querySelectorAll('button')].find(node=>node.textContent===label);
        assert.ok(button, label); button.click(); await tick();
    });
    try {
        await render(); assert.equal(sent.length,0);
        await click('Browser');
        assert.equal(dom.window.document.querySelector('[aria-expanded]')?.getAttribute('aria-expanded'),'true');
        assert.ok(dom.window.document.querySelector('video'));
        assert.ok(sent.some(item=>item.operation.action==='start'), 'one click opens and connects the viewer');
        assert.equal(sent.find(item=>item.operation.action==='start').operation.expected_revision,7);
        tab = {...tab,snapshot:{revision:9}}; await render();
        assert.equal(peers.length,1, 'snapshot refresh does not remount the viewer');
        assert.ok(sent.every(item=>item.name==='first' && item.session_id==='session-one' && item.incarnation==='owner-one'));
        assert.equal(peers.length,1);
        client = socket('replacement'); await render();
        assert.ok(sent.some(item=>item.name==='first' && item.operation.action==='detach')); assert.equal(peers[0].closed,true);
        tab = {...tab,stale:true}; client = null; await render();
        assert.match(dom.window.document.body.textContent!,/Browser detached/);
        assert.equal(sent.at(-1).name,'replacement'); assert.equal(sent.at(-1).operation.action,'detach');
        client = socket('second'); tab = {...tab,key:'two',vessel:'vessel-two',session:'session-two',incarnation:'owner-two',stale:false}; await render();
        assert.equal(dom.window.document.querySelector('video'),null,'switching tasks requires explicit reopening');
        await click('Browser');
        assert.equal(sent.at(-1).session_id,'session-two'); assert.equal(sent.at(-1).incarnation,'owner-two');
        await click('Browser'); assert.equal(sent.at(-1).operation.action,'detach');
        await click('Browser');
        await act(async()=>{dom.window.document.dispatchEvent(new dom.window.KeyboardEvent('keydown',{key:'Escape',bubbles:true}));await tick();});
        assert.equal(dom.window.document.activeElement?.textContent,'Browser','Escape restores the discoverable action');
        await click('Browser'); await click('Close ×');
        assert.equal(dom.window.document.querySelector('video'),null);
        await click('Browser');
        await act(async()=>root.unmount()); await tick();
        assert.equal(sent.at(-1).operation.action,'detach');
        assert.ok(!sent.some(item=>item.operation.action==='close'));
        assert.ok(peers.every(peer=>peer.closed));
        assert.equal(dom.window.localStorage.length,0);
    } finally {
        await act(async()=>root.unmount());
        names.forEach((name,index)=>{const descriptor=saved[index];if(descriptor)Object.defineProperty(globalThis,name,descriptor);else delete (globalThis as any)[name];});
        dom.window.close();
    }
});


test('browser activity uses current voyage tool metadata, not assistant prose', () => {
    assert.equal(browserActivity({messages:[{role:'assistant',content:'Try host_browser'}]}),false);
    assert.equal(browserActivity({messages:[{role:'assistant',tool_calls:[{id:'call',function:{name:'host_browser'}}]}]}),true);
    assert.equal(browserActivity({messages:[{role:'tool',name:'functions.host_browser',content:'result'}]}),true);
    assert.equal(browserActivity({messages:[],run:{tool_previews:[{name:'host_browser'}]}}),true);
    assert.equal(browserActivity({messages:[{role:'tool',name:'shell'}]}),false);
    assert.equal(browserActivity(null),false);
});
