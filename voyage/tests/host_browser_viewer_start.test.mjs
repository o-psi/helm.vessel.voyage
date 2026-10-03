// Selected connection lifecycle only; no browser/provider/network process.
import test from 'node:test';
import assert from 'node:assert/strict';
import {gzipSync} from 'node:zlib';
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
   if(operation.action==='detach')current={...current,binding:{...binding}};
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

test('explicit Disconnect then Check uses fresh Status and reattaches without Start',async()=>{
 const f=fixture({running:true});await f.session.connect({start:false});
 const before=f.sent.filter(op=>op.action==='attach').length;f.session.disconnect();
 await new Promise(resolve=>setImmediate(resolve));await f.session.recover();
 assert.equal(f.sent.filter(op=>op.action==='attach').length,before+1);
 assert.equal(f.sent.filter(op=>op.action==='start').length,0);f.session.dispose();
});

test('idle background refresh leaves explicit Disconnect visible until Check reattaches',async()=>{
 const f=fixture({running:true});
 try{
  await f.session.connect({start:false});const before=f.sent.filter(op=>op.action==='attach').length;
  f.session.disconnect();await new Promise(resolve=>setImmediate(resolve));const sent=f.sent.length;
  // These are the same five interval callbacks invoked over the ten-second idle.
  for(let i=0;i<5;i++){await f.session.refresh();f.session.schedulePoll(0);await new Promise(resolve=>setImmediate(resolve));}
  assert.equal(f.sent.length,sent);assert.equal(f.session.phase,'disconnected');assert.equal(f.session.issue,null);assert.equal(f.session.pollTimer,null);
  await f.session.recover();assert.equal(f.session.attached,true);assert.equal(f.session.status.binding.browser_id,'browser');
  assert.equal(f.sent.filter(op=>op.action==='attach').length,before+1);assert.equal(f.sent.filter(op=>op.action==='start').length,0);
 }finally{f.session.dispose();}
});

test('a status reply pending across Disconnect cannot replace its recovery state',async()=>{
 const f=fixture({running:true});let release,entered;
 try{
  await f.session.connect({start:false});const gate=new Promise(r=>{release=r;}),started=new Promise(r=>{entered=r;}),transport=f.session.transport;
  f.session.transport=async op=>{if(op.action==='status'){entered();await gate;}return transport(op);};
  const pending=f.session.refresh();await started;f.session.disconnect();release();await pending;
  assert.equal(f.session.phase,'disconnected');assert.equal(f.session.issue,null);assert.equal(f.session.pollTimer,null);
  f.session.transport=transport;await f.session.recover();assert.equal(f.session.attached,true);assert.equal(f.sent.filter(op=>op.action==='start').length,0);
 }finally{release?.();f.session.dispose();}
});

test('an in-flight mirror poll neither fails nor reschedules after Disconnect',async()=>{
 const f=fixture({running:true});let release;
 try{
  await f.session.connect({start:false});let entered;
  const gate=new Promise(r=>{release=r;}),started=new Promise(r=>{entered=r;}),transport=f.session.transport;
  f.session.transport=async op=>{if(op.action==='mirror'){entered();await gate;}return transport(op);};
  f.session.schedulePoll(0);await started;f.session.disconnect();release();
  while(f.session.polling)await new Promise(resolve=>setImmediate(resolve));
  assert.equal(f.session.phase,'disconnected');assert.equal(f.session.issue,null);assert.equal(f.session.pollTimer,null);
 }finally{release?.();f.session.dispose();}
});

test('a late explicit Check reply cannot replace a newer Disconnect',async()=>{
 const f=fixture({running:true});let release,entered;
 try{
  await f.session.connect({start:false});f.session.disconnect();await new Promise(resolve=>setImmediate(resolve));
  const gate=new Promise(r=>{release=r;}),started=new Promise(r=>{entered=r;}),transport=f.session.transport;
  f.session.transport=async op=>{if(op.action==='status'){entered();await gate;}return transport(op);};
  const recovering=f.session.recover();await started;f.session.disconnect();release();await recovering;
  assert.equal(f.session.phase,'disconnected');assert.equal(f.session.issue,null);assert.equal(f.session.pollTimer,null);
 }finally{release?.();f.session.dispose();}
});

test('Disconnect during asynchronous top or child decoding cannot rebuild retired replay DOM',async()=>{
 const NativeResponse=globalThis.Response;
 for(const child of [false,true]){
  const f=fixture({running:true});let release,entered,rebuilds=0,visuals=0;
  try{
   await f.session.connect({start:false});clearTimeout(f.session.pollTimer);f.session.pollTimer=null;
   const gate=new Promise(r=>{release=r;}),started=new Promise(r=>{entered=r;});
   globalThis.Response=class extends NativeResponse {async text(){const text=await super.text();entered();await gate;return text;}};
   const value={encoding:'gzip',data_base64:gzipSync(JSON.stringify([{type:2,data:{node:{id:1}}}])).toString('base64'),reset:true,cursor:1,latest:1,visuals:[]};
   f.session.newPlayer=()=>{rebuilds++;return {addEvent(){},destroy(){}};};f.session.onVisuals=()=>{visuals++;};
   let pending;
   if(child){f.session.frameLayer={replaceChildren(){}};f.session.replayer={destroy(){}};pending=f.session.updateFrames([{...value,frame_id:'child',host_node_id:1}]);}
   else {const transport=f.session.transport;f.session.transport=async op=>({...await transport(op),value:op.action==='mirror'?value:null});pending=f.session.pull();}
   await started;f.session.disconnect();const clearedVisuals=visuals;release();await pending;
   assert.equal(rebuilds,0);assert.equal(visuals,clearedVisuals);assert.equal(f.session.frames.size,0);assert.equal(f.session.phase,'disconnected');
  }finally{release?.();globalThis.Response=NativeResponse;f.session.dispose();}
 }
});
