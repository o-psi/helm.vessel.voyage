import test from 'node:test';
import assert from 'node:assert/strict';
import {webcrypto} from 'node:crypto';
import {SharedDraft,updateText,imageBytes} from '../resources/js/shared-drafts.js';
import {validDraftOperation,validCommand} from '../gateway/protocol.js';
globalThis.crypto ||= webcrypto;
const id='11111111-1111-4111-8111-111111111111';
const storage=()=>{const map=new Map();return {getItem:k=>map.get(k)||null,setItem:(k,v)=>map.set(k,v),removeItem:k=>map.delete(k)};};
function fixture(){
 let record={draft_id:id,revision:1,document:{target:{type:'new_chat',workspace:'/repo'},parts:[{type:'text',text:'original'}]}};
 const receipts=new Map();let offline=false,unknown=false;
 const client={async exchange({command:{operation:o}}){
   if(offline)throw Error('offline');let result;
   if(receipts.has(o.command_id))result=receipts.get(o.command_id);
   else if(o.op==='get')result=record;
   else if(o.op==='put'){
     if(o.expected_revision!==(record?.revision||0))return {protocol:1,error:'conflict',outcome_unknown:false};
     record={draft_id:o.draft_id,revision:(record?.revision||0)+1,document:structuredClone(o.document)};result=record;receipts.set(o.command_id,result);
   }else if(o.op==='delete'){
     if(o.expected_revision!==record?.revision)return {protocol:1,error:'conflict',outcome_unknown:false};record=null;result={deleted:true};
   }
   if(unknown){unknown=false;throw Error('lost reply');}
   return {protocol:1,error:null,outcome_unknown:false,result:structuredClone(result)};
 }};
 const make=()=>new SharedDraft({client:()=>client,storage:storage(),key:'draft'});
 return {make,client,get record(){return record;},set offline(v){offline=v;},set unknown(v){unknown=v;}};
}
test('two independent clients discover and exchange text without execution',async()=>{
 const f=fixture(),a=f.make(),b=f.make();await a.open(f.record);await b.open(f.record);
 a.edit([{type:'text',text:'from phone'}]);await a.save();await b.poll();assert.equal(b.document.parts[0].text,'from phone');
 b.edit([{type:'text',text:'from terminal'}]);await b.save();await a.poll();assert.equal(a.document.parts[0].text,'from terminal');
});
test('CAS conflict retains both edits and demands explicit choice',async()=>{
 const f=fixture(),a=f.make(),b=f.make();await a.open(f.record);await b.open(f.record);a.edit([{type:'text',text:'a'}]);b.edit([{type:'text',text:'b'}]);await a.save();await assert.rejects(b.save());assert.equal(b.document.parts[0].text,'b');assert.equal(b.conflict.document.parts[0].text,'a');
});
test('offline recovery and uncertain save reuse exact identity',async()=>{
 const f=fixture(),a=f.make();await a.open(f.record);a.edit([{type:'text',text:'recover'}]);f.offline=true;await assert.rejects(a.save());const command=a.pending.command_id;f.offline=false;f.unknown=true;await assert.rejects(a.save());assert.equal(a.pending.command_id,command);await a.save();assert.equal(a.record.revision,2);assert.equal(a.dirty,false);
});
test('reload restores unsaved edits and protects post-send changes',async()=>{
 const f=fixture(),a=f.make();await a.open(f.record);const sent=structuredClone(a.record);a.edit([{type:'text',text:'later'}]);const b=new SharedDraft({client:()=>f.client,storage:a.storage,key:a.key});await b.open(f.record);assert.equal(b.document.parts[0].text,'later');assert.equal(await a.sent(sent),false);assert.ok(f.record);await a.save();assert.equal(await a.sent(a.record),true);assert.deepEqual(f.record.document.parts,[]);
});
test('remote edit after admission blocks stale deletion',async()=>{const f=fixture(),a=f.make(),b=f.make();await a.open(f.record);await b.open(f.record);b.edit([{type:'text',text:'other device'}]);await b.save();await assert.rejects(a.sent(a.record));assert.equal(f.record.document.parts[0].text,'other device');});
test('ordered images remain ordered through composer text edits',()=>{const images=[{type:'image',attachment:{id:'a'}},{type:'image',attachment:{id:'b'}}];assert.deepEqual(updateText([{type:'text',text:'before'},images[0],{type:'text',text:'after'},images[1]],'edited'),[{type:'text',text:'edited'},images[0],{type:'text',text:''},images[1]]);});
test('gateway admits bounded draft/content operations only',()=>{
 assert.equal(validDraftOperation({op:'put',command_id:id,draft_id:id,expected_revision:0,document:{target:{type:'steer',session_id:id,run_id:id,incarnation:id},parts:[{type:'text',text:'hi'}]}}),true);
 assert.equal(validDraftOperation({op:'upload_image',command_id:id,draft_id:id,name:'image',data_base64:'a'.repeat(2796205)}),false);
 assert.equal(validDraftOperation({op:'get',draft_id:id,path:'/private'}),false);
 assert.equal(validCommand({type:'command',request_id:id,request:{protocol:1,command:{op:'submit_content',session_id:id,command_id:id,expected_revision:1,expires_at_ms:100,content:[{type:'text',text:'hi'}]}}}),true);
});
test('authorized raster bytes checked for size and integrity; SVG refused',async()=>{
 const bytes=new Uint8Array([1,2,3]),hash=Buffer.from(await crypto.subtle.digest('SHA-256',bytes)).toString('hex');
 const client={exchange:async()=>({protocol:1,error:null,outcome_unknown:false,result:{offset:0,data_base64:'AQID'}})};
 const attachment={id,media_type:'image/png',byte_size:3,sha256:hash};assert.equal((await imageBytes(client,attachment,{draft_id:id})).size,3);
 await assert.rejects(imageBytes(client,{...attachment,sha256:'0'.repeat(64)},{draft_id:id}),/integrity/);
 await assert.rejects(imageBytes(client,{...attachment,media_type:'image/svg+xml'},{draft_id:id}),/Unsupported/);
});

test('restored text/image/text edits preserve text on both sides',()=>{const image={type:'image',attachment:{id}};assert.deepEqual(updateText([{type:'text',text:'before'},image,{type:'text',text:'after'}],'BEFOREafter'),[{type:'text',text:'BEFORE'},image,{type:'text',text:'after'}]);assert.deepEqual(updateText([{type:'text',text:'before'},image,{type:'text',text:'after'}],'beforeAFTER'),[{type:'text',text:'before'},image,{type:'text',text:'AFTER'}]);});

test('later local edit while clear in flight survives on same stable identity',async()=>{
 const f=fixture(),a=f.make();await a.open(f.record);const original=a.client;let unblock;
 a.client=()=>({exchange:async request=>{await new Promise(resolve=>unblock=resolve);return original().exchange(request);}});
 const sending=a.sent(structuredClone(a.record));await Promise.resolve();a.edit([{type:'text',text:'post-send edit'}]);unblock();assert.equal(await sending,false);assert.equal(a.dirty,true);a.client=original;await a.save();assert.equal(f.record.document.parts[0].text,'post-send edit');assert.equal(f.record.draft_id,id);
});
test('uncertain clear resumes same CAS identity after browser reload',async()=>{
 const f=fixture(),a=f.make();await a.open(f.record);const sent=structuredClone(a.record);f.unknown=true;await assert.rejects(a.sent(sent));const identity=a.deletion.command_id;
 const b=new SharedDraft({client:()=>f.client,storage:a.storage,key:a.key});await b.open(sent);assert.equal(b.deletion.command_id,identity);assert.equal(await b.sent(sent),true);assert.equal(f.record.revision,2);assert.deepEqual(f.record.document.parts,[]);
});
