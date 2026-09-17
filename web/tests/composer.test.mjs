import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {composer} from '../resources/js/composer.js';
const dom=new JSDOM('',{url:'https://fixture.invalid'});
for(const key of ['window','document','Event'])globalThis[key]=dom.window[key];
globalThis.localStorage={getItem(){throw Error('composer accessed storage');},setItem(){throw Error('composer persisted content');}};
function fixture(){
 const root=document.createElement('div');root.innerHTML=`<form id="composer"><div id="prompt"></div><button id="attach-picture" type="button"></button><input id="picture-files" type="file"></form><template id="flux-attachment-panel"><div><div data-images></div></div></template><template id="flux-attachment-image"><div><img><span data-image-name></span><button type="button"></button></div></template>`;
 root.querySelector('#prompt').value='';document.body.append(root);
 let now={vessel:'v',session_id:'a',running:false};const calls=[];
 const fleet={connections:new Map([['v',{client:{async exchange(request){calls.push(request);return {protocol:1,outcome_unknown:false,result:{session_id:request.command.session_id,incarnation:'i',result:{id:'image'}}};}}}]])};
 const api=composer(root,{fleet,current:()=>now,select(vessel,session_id){now={...now,vessel,session_id};},notice:message=>{throw Error(message);}});
 return {root,api,calls,prompt:root.querySelector('#prompt'),setSession(id){now.session_id=id;}};
}
test('text lives only in memory, selection restores it, remount does not',async()=>{
 const f=fixture();f.prompt.value='unsent';f.prompt.dispatchEvent(new Event('input'));f.api.remember();f.setSession('b');await f.api.select();assert.equal(f.prompt.value,'');f.setSession('a');await f.api.select();assert.equal(f.prompt.value,'unsent');assert.deepEqual(f.calls,[]);const fresh=fixture();await fresh.api.select();assert.equal(fresh.prompt.value,'');
});
test('admission does not clear edits made after sending',async()=>{
 const f=fixture();f.prompt.value='first';const sent=await f.api.beforeSend();f.prompt.value='second';f.prompt.dispatchEvent(new Event('input'));await f.api.admitted(sent);assert.equal(f.prompt.value,'second');const second=await f.api.beforeSend();await f.api.admitted(second);assert.equal(f.prompt.value,'');assert.deepEqual(f.calls,[]);
});
test('new chat keeps its composer through session creation, without a draft RPC',async()=>{
 const f=fixture();await f.api.newChat('v','/repo');f.prompt.value='hello';const sent=await f.api.beforeSend();const origin=f.api.capture();assert.equal(await f.api.created('v','new','/repo',origin),true);assert.equal(f.api.active,sent.draft);assert.equal(f.prompt.value,'hello');assert.deepEqual(f.calls,[]);
});
test('pictures upload directly to the session and reuse uncertain upload identity',async()=>{
 const f=fixture();f.prompt.value='picture';const sent=await f.api.beforeSend();const image={type:'image',name:'x.png',data_base64:'eA==',uploads:new Map()};sent.record.document.parts.push(image);const content=await f.api.promote(sent,'a');assert.equal(content[1].attachment.id,'image');await f.api.promote(sent,'a');assert.equal(f.calls.length,1);assert.equal(f.calls[0].command.op,'upload_image');assert.equal(f.calls[0].command.session_id,'a');
});

test('new-voyage location edits and reentry retain unsent text without RPCs',async()=>{
 const f=fixture();await f.api.newChat('v','/one');f.prompt.value='keep this task';f.prompt.dispatchEvent(new Event('input'));
 const key=f.api.capture().key;await f.api.newChat('v','/two');assert.equal(f.prompt.value,'keep this task');assert.equal(f.api.capture().key,key);assert.equal(f.api.capture().workspace,'/two');
 f.api.remember();f.setSession('other');await f.api.select();assert.equal(f.prompt.value,'');assert.equal(f.api.newDraft.document.target.workspace,'/two');
 await f.api.newChat('v','/two');assert.equal(f.prompt.value,'keep this task');assert.deepEqual(f.calls,[]);
});
