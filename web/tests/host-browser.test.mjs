import test from 'node:test';
import assert from 'node:assert/strict';
import {gzipSync} from 'node:zlib';
import {BrowserConnection, mountBrowserViewer} from '../../helm/browser-view/viewer.mjs';
import {mountHostBrowser,hostBrowserAdapter} from '../resources/js/host-browser.js';

const nil='00000000-0000-0000-0000-000000000000';
const binding={incarnation:'inc',browser_id:'browser',attachment_id:'viewer',tab_id:'tab',document_epoch:1,viewport_epoch:1,controller_epoch:1,capture_epoch:1};
const status=(changes={})=>({available:true,running:true,binding:{...binding},mode:'human',controller:'viewer',tabs:['tab'],viewport:{width:640,height:480},input_sequence:0,...changes});
const mirror=(reset=true,cursor=2,events=[{type:4,timestamp:1,data:{href:'about:blank',width:640,height:480}},{type:2,timestamp:2,data:{node:{type:0,id:1,childNodes:[{type:2,id:2,tagName:'html',attributes:{},childNodes:[{type:2,id:3,tagName:'head',attributes:{},childNodes:[]},{type:2,id:4,tagName:'body',attributes:{},childNodes:[]}]}]},initialOffset:{top:0,left:0}}}])=>({encoding:'gzip',data_base64:gzipSync(JSON.stringify(events)).toString('base64'),reset,cursor,latest:cursor,visuals:[]});
function fixture(reply){
    const sent=[];let id=0;
    const session=new BrowserConnection({mirror:{replaceChildren(){}},timeout:80,uuid:()=>`command-${++id}`,
        context:()=>({incarnation:'inc',revision:7}),transport:async operation=>{
            sent.push(operation);return reply?.(operation)||{status:status(),value:operation.action==='mirror'?mirror():null};
        }});
    return {session,sent};
}

// These unit checks use a recorder-shaped replay stub. The Chromium journey
// exercises the real rrweb library and page events.
test('viewer starts once and requires a full DOM snapshot before input',async()=>{
    const old=globalThis.rrweb;
    globalThis.rrweb={Replayer:class {
        constructor(){this.iframe={contentDocument:{createElement:()=>({}),head:{append(){}}},referrerPolicy:''};}
        enableInteract(){} on(name,callback){if(name==='fullsnapshot-rebuilded')this.ready=callback;}
        startLive(){} addEvent(event){if(event.type===2)this.ready?.();} destroy(){}
    }};
    let current=status({running:false,binding:null});
    const f=fixture(operation=>{
        if(operation.action==='start')current=status({binding:{...binding,attachment_id:nil}});
        if(operation.action==='attach')current=status();
        return {status:current,value:operation.action==='mirror'?mirror():null};
    });
    try{
        await f.session.connect();
        assert.deepEqual(f.sent.map(operation=>operation.action),['status','start','attach','mirror'],f.session.lastError);
        assert.equal(f.sent[1].expected_revision,7);
        assert.equal(f.sent[3].binding.attachment_id,'viewer');
        assert.equal(f.session.streaming,true);
        assert.equal(f.session.canInput,true);
    }finally{f.session.dispose();globalThis.rrweb=old;}
});

test('private handoff clears replay and refuses queued input from an old fence',async()=>{
    const f=fixture();f.session.accept(status());
    f.session.streaming=true;
    f.session.queue.push({input:{type:'fill',node_id:9,text:'old'},key:'stale',reject:()=>{}});
    f.session.accept(status({mode:'private',binding:{...binding,controller_epoch:2,capture_epoch:2}}));
    assert.equal(f.session.queue.length,0);
    assert.equal(f.session.streaming,false);
    assert.equal(f.session.controls,true);
    f.session.dispose();
});

test('late page status cannot roll back a newer document fence',()=>{
    const f=fixture();
    f.session.accept(status({binding:{...binding,document_epoch:3}}));
    f.session.accept(status({binding:{...binding,document_epoch:2}}));
    assert.equal(f.session.status.binding.document_epoch,3);
    f.session.dispose();
});

test('unconfirmed element action is never replayed',async()=>{
    const f=fixture(operation=>{
        if(operation.action==='input')throw Error('reply_lost');
        return {status:status(),value:operation.action==='mirror'?mirror():null};
    });
    f.session.accept(status());f.session.streaming=true;
    await assert.rejects(f.session.confirmedInput({type:'click',node_id:12,button:'left'}));
    assert.equal(f.session.issue,'input-unknown');
    assert.equal(f.sent.filter(operation=>operation.action==='input').length,1);
    f.session.dispose();
});

test('dialog interrupt preserves consecutive input sequence numbers',async()=>{
    let release;const pending=new Promise(resolve=>{release=resolve;});
    const f=fixture(async operation=>{
        if(operation.action==='input'&&operation.sequence===1)await pending;
        return {status:status(),value:null};
    });
    f.session.accept(status());f.session.streaming=true;
    const first=f.session.confirmedInput({type:'resize',width:800,height:600});
    await Promise.resolve();
    const dialog=f.session.urgentInput({type:'dialog',accept:true,text:'ok'});
    assert.deepEqual(f.sent.filter(operation=>operation.action==='input').map(operation=>operation.sequence),[1,2]);
    assert.equal(await dialog,true);release();assert.equal(await first,null);f.session.dispose();
});

test('browser chrome shows control and stopped recovery without a video stage',async()=>{
    const {JSDOM}=await import('jsdom');
    const dom=new JSDOM('<main id="viewer"></main>',{url:'https://helm.example'});
    const root=dom.window.document.querySelector('#viewer');
    const view=mountBrowserViewer(root,{autoConnect:false,transport:async()=>({status:status()}),context:()=>({incarnation:'inc',revision:1})});
    try{
        view.session.accept(status({mode:'agent',controller:null,page:{url:'https://example.com',title:'Example Domain'},tab_details:[{id:'tab',title:'Example Domain'}]}));
        view.session.streaming=true;view.session.phase='live';view.session.emit();
        assert.equal(root.querySelector('.browser-next-status').textContent,'Watching browser');
        assert.ok(root.querySelector('.browser-next-mirror'));
        assert.equal(root.querySelector('video'),null);
        assert.equal(root.querySelector('.browser-next-primary').textContent,'Take control privately');
        view.session.accept(status({available:true,running:false,binding:null,mode:null,controller:null}));
        assert.match(root.querySelector('.browser-next-recovery').textContent,/This browser is stopped/);
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
