import test from 'node:test';
import assert from 'node:assert/strict';
import {BrowserSession, videoPoint} from '../../helm/browser-view/viewer.mjs';
import {mountHostBrowser, hostBrowserAdapter} from '../resources/js/host-browser.js';
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

function adapterFixture(replies) {
    const sent=[];
    const client={exchange:async request=>{sent.push(structuredClone(request.command));const next=replies.shift(); if(next instanceof Error) throw next; return next;}};
    return {sent,...hostBrowserAdapter({client,sessionId:'session',incarnation:'old',context:()=>({incarnation:'old',revision:1})})};
}
const envelope = (result, incarnation='new', extra={}) => ({protocol:1,error:null,outcome_unknown:false,result:{session_id:'session',incarnation,result},...extra});
const preparation = () => envelope({status:'prepared',not_dispatched:true});
test('prepared Start refreshes fences and resends exact non-admitted intent once',async()=>{
    const a=adapterFixture([preparation(),envelope({revision:9}),envelope({status:status()})]);
    const original={action:'start',command_id:'same-id',incarnation:'old',expected_revision:1};
    await a.transport(original);
    assert.deepEqual(a.sent.map(c=>c.op),['host_browser','snapshot','host_browser']);
    assert.deepEqual(a.sent[2].operation,{...original,incarnation:'new',expected_revision:9});
    assert.deepEqual(original,{action:'start',command_id:'same-id',incarnation:'old',expected_revision:1});
    assert.deepEqual(a.context(),{incarnation:'new',revision:9});
});
test('prepared Status refreshes context for subsequent Start',async()=>{
    const a=adapterFixture([preparation(),envelope({revision:9}),envelope({status:status()})]);
    await a.transport({action:'status'});
    assert.deepEqual(a.sent[2].operation,{action:'status'});
    assert.deepEqual(a.context(),{incarnation:'new',revision:9});
});
test('uncertainty, wrong identity and incomplete preparation never authorize resend',async()=>{
    for(const reply of [new Error('lost'),envelope({},'new'),envelope({status:'prepared'}),envelope({not_dispatched:true}),envelope({status:'prepared',not_dispatched:true},'new',{outcome_unknown:true}),{...preparation(),result:{...preparation().result,session_id:'other'}}]) {
        const a=adapterFixture([reply]);
        await assert.rejects(a.transport({action:'status'})); assert.equal(a.sent.length,1);
    }
});
test('bound effects never prepare; repeated preparation is bounded',async()=>{
    const a=adapterFixture([preparation()]);
    await assert.rejects(a.transport({action:'control',binding:{...binding,incarnation:'old'},mode:'human',command_id:'id'}));
    assert.equal(a.sent.length,1);
    const b=adapterFixture([preparation(),envelope({revision:9}),preparation()]);
    await assert.rejects(b.transport({action:'status'})); assert.equal(b.sent.length,3);
});
test('snapshot fence/revision failures and uncertain resend do not retry',async()=>{
    for(const snapshot of [envelope({revision:9},'third'),envelope({revision:-1}),envelope({}),new Error('snapshot lost')]) {
        const a=adapterFixture([preparation(),snapshot]);
        await assert.rejects(a.transport({action:'status'})); assert.equal(a.sent.length,2);
        assert.equal(a.context().incarnation,'old');
    }
    const a=adapterFixture([preparation(),envelope({revision:9}),new Error('lost after admission')]);
    await assert.rejects(a.transport({action:'start',incarnation:'old',expected_revision:1,command_id:'id'}));
    assert.equal(a.sent.length,3);
});

test('website dialog can interrupt blocked pointer input without replay', async()=>{
    let release;
    const {session,sent} = fixture(op => op.action === 'input' && op.input.type === 'pointer' ? new Promise(r=>release=r) : Promise.resolve({status:status()}));
    session.accept(status());session.streaming=true;
    session.input({type:'pointer',x:1,y:1,button:'left',pressed:false});await tick();
    session.input({type:'dialog',accept:true,text:null});await tick();
    assert.deepEqual(sent.map(x=>x.input.type),['pointer','dialog']);
    assert.deepEqual(sent.map(x=>x.sequence),[1,2]);
    release({status:status()});await tick();session.dispose();
});

test('remounted viewer continues acknowledged attachment input sequence',async()=>{
    const {session,sent} = fixture(op=>({status:status({input_sequence:42}),value:op.signal?.type==='request_offer'?{type:'offer',sdp:'offer'}:null}));
    await session.connect();session.streaming=true;
    session.input({type:'text',text:'synthetic'});await tick();
    assert.equal(sent.find(op=>op.action==='input').sequence,43);
    session.dispose();
});

test('authorized offer can provide viewer-only RTC configuration',async()=>{
    const rtc={iceServers:[{urls:['turn:relay.invalid:3478'],username:'fixture',credential:'synthetic'}],iceTransportPolicy:'relay'};
    let seen;
    const f=fixture(op=>({status:status(),value:op.signal?.type==='request_offer'?{type:'offer',sdp:'offer',rtc_configuration:rtc}:null}));
    const original=f.session.peer;f.session.peer=config=>{seen=config;return original(config);};
    await f.session.connect();assert.deepEqual(seen,rtc);f.session.dispose();
});
