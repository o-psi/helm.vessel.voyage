import test from 'node:test';
import assert from 'node:assert/strict';
import {BrowserSession, videoPoint} from '../../helm/browser-view/viewer.mjs';
import {mountHostBrowser} from '../resources/js/host-browser.js';
const nil = '00000000-0000-0000-0000-000000000000';
const binding = {incarnation:'inc',browser_id:'browser',attachment_id:'viewer',tab_id:'tab',document_epoch:1,viewport_epoch:1,controller_epoch:1,capture_epoch:1};
const status = (changes = {}) => ({available:true,running:true,binding:{...binding},mode:'human',controller:'viewer',tabs:['tab'],...changes});
const tick = () => new Promise(resolve => setTimeout(resolve,0));
function fixture(transport) {
    const sent = [], peers = [], video = {srcObject:null,play:() => Promise.resolve()};
    const peer = () => {
        const pc = {iceGatheringState:'complete',connectionState:'new',close(){this.closed=true;},async setRemoteDescription(value){this.remote=value;},async createAnswer(){return {type:'answer',sdp:'answer'};},async setLocalDescription(value){this.localDescription=value;}};
        peers.push(pc); return pc;
    };
    let id = 0;
    const session = new BrowserSession({video,peer,context:() => ({incarnation:'inc',revision:7}),uuid:() => `command-${++id}`,transport:async op => {sent.push(op); return transport ? transport(op) : {status:status(),value:op.signal?.type === 'request_offer' ? {type:'offer',sdp:'offer'} : null};}});
    return {session,sent,peers,video};
}
test('complete-ICE offer/answer uses actual peer video and full binding',async () => {
    const {session,sent,peers,video} = fixture();
    await session.connect();
    assert.deepEqual(sent.map(op=>op.action),['status','signal','signal']);
    assert.equal(sent[1].signal.type,'request_offer');
    assert.deepEqual(sent[2].signal,{type:'answer',sdp:'answer'});
    assert.deepEqual(sent[2].binding,binding);
    const stream = {getTracks:()=>[]}; peers[0].ontrack({streams:[stream],track:{stop(){}}});
    assert.equal(video.srcObject,stream); assert.equal(session.streaming,true);
    session.dispose();
});
test('start and attach use authoritative assigned attachment before offer', async () => {
    let current = status({running:false,binding:null});
    const {session,sent} = fixture(op => {
        if(op.action==='start') current=status({binding:{...binding,attachment_id:nil}});
        if(op.action==='attach') current=status();
        return {status:current,value:op.signal?.type==='request_offer'?{type:'offer',sdp:'offer'}:null};
    });
    await session.connect();
    assert.deepEqual(sent.map(op=>op.action),['status','start','attach','signal','signal']);
    assert.equal(sent[1].expected_revision,7); assert.notEqual(sent[2].binding.attachment_id,nil);
    assert.equal(sent[3].binding.attachment_id,'viewer'); session.dispose();
});
test('late peer callbacks cannot display video after disconnect',async () => {
    const {session,peers,video} = fixture(); await session.connect();
    const late = peers[0].ontrack; session.disconnect(); let stopped=false;
    late({track:{stop(){stopped=true;}},streams:[{}]});
    assert.equal(video.srcObject,null); assert.equal(stopped,true); assert.equal(peers[0].closed,true);
});
test('pending request cannot restore state after disconnect',async () => {
    let resolve; const {session} = fixture(() => new Promise(done=>{resolve=done;}));
    const pending=session.connect(); session.disconnect(); resolve({status:status()}); await pending;
    assert.equal(session.status,null); assert.equal(session.pc,null);
});
test('privacy transition clears video and queued input before renegotiating',async () => {
    const {session,peers,video,sent} = fixture(op=>({status:status({mode:op.action==='control'?'private':'human'}),value:op.signal?.type==='request_offer'?{type:'offer',sdp:'offer'}:null}));
    await session.connect(); video.srcObject={getTracks:()=>[]}; session.queue.push({});
    const pending=session.control('private'); assert.equal(video.srcObject,null); assert.equal(session.queue.length,0); await pending;
    assert.equal(peers[0].closed,true); assert.equal(sent.find(op=>op.action==='control').mode,'private'); session.dispose();
});
test('inputs are consecutive and stale queued input is dropped after epoch change',async () => {
    let resolve; const {session,sent} = fixture(op => op.action==='input' ? new Promise(done=>{resolve=done;}) : {status:status()});
    session.accept(status()); session.streaming=true;
    session.input({type:'key',key:'a',pressed:true}); session.input({type:'key',key:'a',pressed:false});
    resolve({status:status({binding:{...binding,document_epoch:2}})}); await tick();
    assert.equal(sent.length,1); assert.equal(sent[0].sequence,1); assert.equal(session.queue.length,0);
    assert.equal(session.streaming,true); session.dispose();
});
test('agent and non-controller cannot send input; slow queue fences immediately',async () => {
    const {session} = fixture(()=>new Promise(()=>{})); session.accept(status({mode:'agent'})); session.streaming=true;
    assert.equal(session.input({type:'text',text:'secret'}),false);
    session.accept(status({controller:'other'})); session.streaming=true; assert.equal(session.input({type:'text',text:'secret'}),false);
    session.accept(status()); session.streaming=true; session.timeout=10;
    for(let i=0;i<35;i++) session.input({type:'text',text:'x'});
    assert.equal(session.status,null); assert.equal(session.queue.length,0); await new Promise(resolve=>setTimeout(resolve,20)); session.dispose();
});
test('video source coordinates account for letterboxing and reject outside points',()=>{
    const video={videoWidth:1280,videoHeight:720,getBoundingClientRect:()=>({left:10,top:20,width:640,height:640})};
    assert.equal(videoPoint(video,12,22),null);
    assert.deepEqual(videoPoint(video,330,340),{x:640,y:360});
});
test('web adapter is available without creating another transport',()=>{assert.equal(typeof mountHostBrowser,'function');});

test('mounted Web UI sends transient navigation/text/dialog/resize/tab and clears on disposal',async () => {
    const {JSDOM} = await import('jsdom');
    const dom = new JSDOM('<section id="viewer"></section>',{url:'https://helm.example'});
    const root = dom.window.document.querySelector('section'), sent=[];
    const client={exchange:async request => { sent.push(request); return {protocol:1,error:null,outcome_unknown:false,result:{session_id:'session',incarnation:'inc',result:{status:status()}}}; }};
    const viewer=mountHostBrowser(root,{client,sessionId:'session',incarnation:'inc',context:()=>({incarnation:'inc',revision:1})});
    try {
        viewer.session.accept(status()); viewer.session.streaming=true; viewer.session.notify();
        const input=[]; viewer.session.input=value=>{input.push(value);return true;};
        const click=label=>[...root.querySelectorAll('button')].find(node=>node.textContent===label).click();
        const address=root.querySelector('input[type=url]'); address.value='https://example.com'; root.querySelector('form').dispatchEvent(new dom.window.Event('submit',{cancelable:true}));
        const text=root.querySelector('textarea'); text.value='日本語'; click('Send text');
        root.querySelector('input:not([type=url])').value='private response'; click('Accept dialog'); click('Resize'); click('New tab'); click('Tab 1'); click('Close tab 1');
        assert.deepEqual(input.map(value=>value.type),['navigate','text','dialog','resize','tab','tab','tab']);
        assert.equal(text.value,''); assert.equal(address.value,'');
        await viewer.session.transport({action:'status'});
        assert.deepEqual(sent[0].command,{op:'host_browser',session_id:'session',incarnation:'inc',operation:{action:'status'}});
        viewer.dispose(); assert.equal(root.children.length,0);
    } finally { viewer.dispose(); dom.window.close(); }
});
