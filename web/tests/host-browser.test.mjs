import test from 'node:test';
import assert from 'node:assert/strict';
import {BrowserConnection, screenPoint, mountBrowserViewer} from '../../helm/browser-view/viewer.mjs';
import {mountHostBrowser,hostBrowserAdapter} from '../resources/js/host-browser.js';

const nil='00000000-0000-0000-0000-000000000000';
const binding={incarnation:'inc',browser_id:'browser',attachment_id:'viewer',tab_id:'tab',document_epoch:1,viewport_epoch:1,controller_epoch:1,capture_epoch:1};
const status=(changes={})=>({available:true,running:true,binding:{...binding},mode:'human',controller:'viewer',tabs:['tab'],viewport:{width:640,height:480},input_sequence:0,...changes});

function fixture({reply=null,frame=true}={}){
    const sent=[],peers=[];
    const track={stopped:false,stop(){this.stopped=true;}};
    const stream={getTracks:()=>[track]};
    const video={srcObject:null,readyState:frame?2:0,videoWidth:frame?640:0,videoHeight:frame?480:0,
        ownerDocument:{hidden:false},addEventListener(){},removeEventListener(){},play:()=>Promise.resolve()};
    const peer=()=>{
        const pc={iceGatheringState:'complete',connectionState:'connected',localDescription:null,closed:false,
            async setRemoteDescription(){queueMicrotask(()=>this.ontrack?.({track,streams:[stream]}));},
            async createAnswer(){return {type:'answer',sdp:'answer'};},
            async setLocalDescription(answer){this.localDescription=answer;},close(){this.closed=true;},};
        peers.push(pc);return pc;
    };
    let id=0;
    const session=new BrowserConnection({video,peer,timeout:80,uuid:()=>`command-${++id}`,
        context:()=>({incarnation:'inc',revision:7}),
        transport:async operation=>{sent.push(operation);return reply?reply(operation):{status:status(),value:operation.signal?.type==='request_offer'?{type:'offer',sdp:'offer'}:null};}});
    return {session,sent,peers,video,track};
}

test('viewer attaches, answers once and waits for decoded pixels',async()=>{
    let current=status({running:false,binding:null});
    const f=fixture({reply:operation=>{
        if(operation.action==='start')current=status({binding:{...binding,attachment_id:nil}});
        if(operation.action==='attach')current=status();
        return {status:current,value:operation.signal?.type==='request_offer'?{type:'offer',sdp:'offer'}:null};
    }});
    await f.session.connect();
    assert.deepEqual(f.sent.map(operation=>operation.action),['status','start','attach','signal','signal']);
    assert.equal(f.sent[1].expected_revision,7);
    assert.equal(f.sent[3].binding.attachment_id,'viewer');
    assert.equal(f.session.streaming,true);
    assert.equal(f.session.phase,'live');
    f.session.dispose();assert.equal(f.peers[0].closed,true);
});

test('private handoff keeps the same video peer and fences queued input',async()=>{
    let privateMode=false;
    const f=fixture({reply:operation=>{
        if(operation.action==='control')privateMode=operation.mode==='private';
        return {status:status(privateMode?{mode:'private',binding:{...binding,controller_epoch:2,capture_epoch:2}}:{}),
            value:operation.signal?.type==='request_offer'?{type:'offer',sdp:'offer'}:null};
    }});
    await f.session.connect();
    f.session.queue.push({input:{type:'text',text:'old'},key:'stale'});
    await f.session.control('private');
    assert.equal(f.session.controls,true);
    assert.equal(f.peers.length,1);
    assert.equal(f.peers[0].closed,false);
    assert.equal(f.session.video.srcObject!==null,true);
    assert.equal(f.session.queue.length,0);
    assert.deepEqual(f.sent.filter(operation=>operation.action==='signal').length,2);
    f.session.dispose();
});

test('unconfirmed input is never replayed and retains the last known browser status',async()=>{
    const f=fixture({reply:operation=>{
        if(operation.action==='input')throw Error('reply_lost');
        return {status:status(),value:operation.signal?.type==='request_offer'?{type:'offer',sdp:'offer'}:null};
    }});
    await f.session.connect();
    await assert.rejects(f.session.confirmedInput({type:'text',text:'private'}));
    assert.equal(f.session.issue,'input-unknown');
    assert.equal(f.session.status.running,true);
    assert.equal(f.session.input({type:'text',text:'again'}),false);
    assert.equal(f.sent.filter(operation=>operation.action==='input').length,1);
    f.session.dispose();
});

test('dialog interrupt bypasses a blocked page action with consecutive sequence numbers',async()=>{
    let release;
    const pending=new Promise(resolve=>{release=resolve;});
    const f=fixture({reply:async operation=>{
        if(operation.action==='input'&&operation.sequence===1)await pending;
        return {status:status(),value:operation.signal?.type==='request_offer'?{type:'offer',sdp:'offer'}:null};
    }});
    await f.session.connect();
    const first=f.session.confirmedInput({type:'resize',width:800,height:600});
    await Promise.resolve();
    const dialog=f.session.urgentInput({type:'dialog',accept:true,text:'ok'});
    assert.deepEqual(f.sent.filter(operation=>operation.action==='input').map(operation=>operation.sequence),[1,2]);
    assert.equal(await dialog,true);
    release();
    assert.equal(await first,true);
    assert.deepEqual(f.sent.filter(operation=>operation.action==='input').map(operation=>operation.sequence),[1,2]);
    f.session.dispose();
});

test('transient ICE disconnect leaves the media peer available for recovery',async()=>{
    const f=fixture();await f.session.connect();
    f.peers[0].connectionState='disconnected';f.peers[0].onconnectionstatechange();
    assert.equal(f.session.phase,'recovering');assert.equal(f.peers[0].closed,false);
    f.peers[0].connectionState='connected';f.peers[0].onconnectionstatechange();
    assert.equal(f.session.phase,'live');assert.equal(f.peers[0].closed,false);
    f.session.dispose();
});

test('returning to a hidden viewer allows fresh frames before declaring a stalled stream',async()=>{
    const f=fixture();await f.session.connect();
    f.video.ownerDocument.hidden=true;f.session.lastFrame=Date.now()-20000;
    await f.session.refresh();
    f.video.ownerDocument.hidden=false;await f.session.refresh();
    assert.equal(f.session.issue,null);assert.equal(f.peers[0].closed,false);
    f.session.dispose();
});

test('a track without decoded pixels cannot enable input',async()=>{
    const f=fixture({frame:false});f.session.timeout=25;
    await f.session.connect();
    assert.equal(f.session.streaming,false);
    assert.equal(f.session.canInput,false);
    assert.equal(f.session.issue,'video-unavailable');
    f.session.dispose();
});

test('screen coordinates map the rendered frame to the CSS browser viewport',()=>{
    const video={videoWidth:320,videoHeight:180,getBoundingClientRect:()=>({left:10,top:20,width:640,height:640})};
    assert.equal(screenPoint(video,12,22,{width:1280,height:720}),null);
    assert.deepEqual(screenPoint(video,330,340,{width:1280,height:720}),{x:640,y:360});
});

test('new browser chrome exposes address, tabs, private control and truthful stopped state',async()=>{
    const {JSDOM}=await import('jsdom');
    const dom=new JSDOM('<main id="viewer"></main>',{url:'https://helm.example'});
    const root=dom.window.document.querySelector('#viewer');
    const view=mountBrowserViewer(root,{autoConnect:false,transport:async()=>({status:status()}),context:()=>({incarnation:'inc',revision:1})});
    try{
        view.session.accept(status({mode:'agent',controller:null,page:{url:'https://example.com',title:'Example Domain',can_go_back:false,can_go_forward:false},tab_details:[{id:'tab',title:'Example Domain'}]}));
        view.session.streaming=true;view.session.phase='live';view.session.emit();
        assert.equal(root.querySelector('.browser-next-status').textContent,'Watching browser');
        assert.ok(root.querySelector('.browser-next-address'));
        assert.equal(root.querySelector('.browser-next-tab button').textContent,'Example Domain');
        assert.equal(root.querySelector('.browser-next-primary').textContent,'Take control privately');
        view.session.accept(status({available:true,running:false,binding:null,mode:null,controller:null}));
        assert.match(root.querySelector('.browser-next-recovery').textContent,/This browser is stopped/);
        assert.equal(root.querySelector('.browser-next-recovery button').textContent,'Start browser');
    }finally{view.dispose();dom.window.close();}
});

test('Web adapter continues to use the authenticated voyage transport',()=>{assert.equal(typeof mountHostBrowser,'function');});
function adapterFixture(replies){
    const sent=[];
    const client={exchange:async request=>{sent.push(structuredClone(request.command));const next=replies.shift();if(next instanceof Error)throw next;return next;}};
    return {sent,...hostBrowserAdapter({client,sessionId:'session',incarnation:'old',context:()=>({incarnation:'old',revision:1})})};
}
const envelope=(result,incarnation='new',extra={})=>({protocol:1,error:null,outcome_unknown:false,result:{session_id:'session',incarnation,result},...extra});
const preparation=()=>envelope({status:'prepared',not_dispatched:true});
test('prepared Start refreshes fences and resends the exact non-admitted intent once',async()=>{
    const adapter=adapterFixture([preparation(),envelope({revision:9}),envelope({status:status()})]);
    const original={action:'start',command_id:'same-id',incarnation:'old',expected_revision:1};
    await adapter.transport(original);
    assert.deepEqual(adapter.sent.map(command=>command.op),['host_browser','snapshot','host_browser']);
    assert.deepEqual(adapter.sent[2].operation,{...original,incarnation:'new',expected_revision:9});
    assert.deepEqual(adapter.context(),{incarnation:'new',revision:9});
});
test('uncertain or bound browser effects never authorize an automatic resend',async()=>{
    const uncertain=adapterFixture([new Error('lost')]);
    await assert.rejects(uncertain.transport({action:'status'}));assert.equal(uncertain.sent.length,1);
    const bound=adapterFixture([preparation()]);
    await assert.rejects(bound.transport({action:'control',binding:{...binding,incarnation:'old'},mode:'human',command_id:'id'}));
    assert.equal(bound.sent.length,1);
});
test('stale conversation reads the current owner before browser attach',async()=>{
    const sent=[];const client={exchange:async request=>{
        sent.push(request.command);
        return {protocol:1,error:null,outcome_unknown:false,result:{session_id:'s',incarnation:'fresh',result:request.command.op==='snapshot'?{session_id:'s',revision:8}:{status:status()}}};
    }};
    const adapter=hostBrowserAdapter({client,sessionId:'s',incarnation:'old',context:()=>({revision:2,refresh:true})});
    await adapter.transport({action:'status'});
    assert.equal(sent[0].op,'snapshot');assert.equal(sent[1].incarnation,'fresh');assert.equal(adapter.context().revision,8);
});
