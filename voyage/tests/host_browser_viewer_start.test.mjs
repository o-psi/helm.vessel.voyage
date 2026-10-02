// Selected connection lifecycle only; no browser/provider/network process.
import test from 'node:test';
import assert from 'node:assert/strict';
import {BrowserConnection} from '../../helm/browser-view/viewer.mjs';
const nil='00000000-0000-0000-0000-000000000000';
const binding={incarnation:'owner',browser_id:'browser',attachment_id:nil,tab_id:'tab',document_epoch:1,viewport_epoch:1,controller_epoch:1,capture_epoch:1};
const status=running=>({available:true,running,binding:running?{...binding}:null,mode:'agent',controller:null});
function fixture({running=false,startOnConnect,read}={}){
 const sent=[];let command=0,current=status(running);
 const session=new BrowserConnection({startOnConnect,mirror:{replaceChildren(){}},timeout:100,
  context:()=>({incarnation:'owner',revision:7}),uuid:()=>`command-${++command}`,
  transport:async operation=>{
   sent.push(operation);
   if(operation.action==='status'&&read)await read();
   if(operation.action==='start')current=status(true);
   if(operation.action==='attach')current={...current,binding:{...current.binding,attachment_id:operation.binding.attachment_id}};
   return {status:current,value:null};
  }});
 return {session,sent};
}
test('incidental reconnect observes a stopped browser without a Start or attachment effect',async()=>{
 const f=fixture({startOnConnect:false});
 try{await f.session.connect();assert.deepEqual(f.sent.map(value=>value.action),['status']);assert.equal(f.session.phase,'stopped');assert.equal(f.session.attached,false);}
 finally{f.session.dispose();}
});
test('incidental reconnect attaches to the same existing browser without creating another',async()=>{
 const f=fixture({running:true,startOnConnect:false});
 try{await f.session.connect();assert.deepEqual(f.sent.map(value=>value.action),['status','attach','mirror']);assert.equal(f.session.status.binding.browser_id,'browser');assert.equal(f.session.attached,true);}
 finally{f.session.dispose();}
});
test('explicit Start overrides observation-only reconnect once with exact current fences',async()=>{
 const f=fixture({startOnConnect:false});
 try{await f.session.connect();await f.session.connect({start:true});assert.deepEqual(f.sent.map(value=>value.action),['status','status','start','attach','mirror']);
  const effect=f.sent.find(value=>value.action==='start');assert.equal(effect.incarnation,'owner');assert.equal(effect.expected_revision,7);assert.equal(effect.command_id,'command-1');}
 finally{f.session.dispose();}
});
test('default native/client connection retains its original normal opening behavior',async()=>{
 const f=fixture();
 try{await f.session.connect();assert.deepEqual(f.sent.map(value=>value.action),['status','start','attach','mirror']);}
 finally{f.session.dispose();}
});
test('disposing an opening viewer during its status read prevents a late Start',async()=>{
 let release;const read=new Promise(resolve=>{release=resolve;});
 const f=fixture({read:()=>read});
 const opening=f.session.connect();
 await Promise.resolve();f.session.dispose();release();await opening;
 assert.deepEqual(f.sent.map(value=>value.action),['status']);assert.equal(f.session.closed,true);
});
