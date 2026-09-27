import test from 'node:test';
import assert from 'node:assert/strict';
import {Workspace} from '../resources/react/workspace.ts';
import {IntentJournal} from '../resources/js/vessel-client.js';

class Storage {
    data = new Map<string, string>();
    get length() { return this.data.size; }
    key(index: number) { return [...this.data.keys()][index] ?? null; }
    getItem(key: string) { return this.data.get(key) ?? null; }
    setItem(key: string, value: string) { this.data.set(key, value); }
    removeItem(key: string) { this.data.delete(key); }
}
function fixture() {
    const storage = new Storage(), commands: any[] = [];
    let mode = 'accepted', subscriptions = 0;
    const response = (session: string, result: unknown) => ({protocol: 1, outcome_unknown: false, result: {session_id: session, incarnation: 'incarnation', result}});
    const client = {
        async exchange({command}: any) {
            commands.push(command);
            if (command.op === 'capabilities') return {protocol:1,outcome_unknown:false,result:{scope:'owner'}};
            if (command.op === 'snapshot') return response(command.session_id, {session_id: command.session_id, revision: 1, observation_cursor: 3, messages: [], run: {state: 'idle'}});
            if (command.op === 'decisions') return response(command.session_id, []);
            if (command.op === 'receipt') return response(command.session_id, {command_id: command.command_id, status: mode});
            if (mode === 'unknown') throw new Error('Disconnected after dispatch');
            return response(command.session_id, {command_id: command.command_id, status: mode});
        },
        subscribe() { subscriptions++; return () => { subscriptions--; }; },
    };
    const connection = {id: 'vessel', name: 'Vessel', client, journal: new IntentJournal(storage, 'tenant:vessel'), voyages: [], status: 'Connected'};
    const workspace = new Workspace(() => new Map([['vessel', connection]]));
    return {workspace, connection, commands, storage, mode: (value: string) => { mode = value; }, subscriptions: () => subscriptions};
}
test('tabs retain isolated drafts and subscriptions while selection changes', async () => {
    const f = fixture();
    const a = f.workspace.open('vessel', 'a', 'A'); await f.workspace.refresh(a);
    f.workspace.draft(a, 'Private draft A');
    const b = f.workspace.open('vessel', 'b', 'B'); await f.workspace.refresh(b);
    f.workspace.draft(b, 'Private draft B');
    assert.equal(f.workspace.open('vessel', 'a', 'A'), a); await f.workspace.refresh(a);
    assert.equal(f.workspace.tabs.get(a)?.draft, 'Private draft A');
    assert.equal(f.workspace.tabs.get(b)?.draft, 'Private draft B');
    assert.equal(f.subscriptions(), 2);
    assert.equal(f.storage.length, 0, 'draft text is not persisted');
    f.workspace.close(); assert.equal(f.subscriptions(), 0);
});
test('uncertain command is journaled once and reconciled without replay', async () => {
    const f = fixture(), key = f.workspace.open('vessel', 'a', 'A'); await f.workspace.refresh(key);
    f.workspace.draft(key, 'Retain me'); f.mode('unknown'); await f.workspace.act(key, 'submit');
    assert.equal(f.commands.filter(c => c.op === 'submit').length, 1);
    assert.equal(f.workspace.tabs.get(key)?.draft, 'Retain me');
    assert.equal(f.workspace.actionable(f.workspace.tabs.get(key)!), false);
    assert.equal([...f.storage.data.values()].some(value => value.includes('Retain me')), false);
    await f.workspace.act(key, 'submit'); assert.equal(f.commands.filter(c => c.op === 'submit').length, 1);
    f.mode('accepted'); await f.workspace.reconcile(key);
    assert.equal(f.storage.length, 0);
    const receipt = f.commands.find(c => c.op === 'receipt');
    assert.equal(receipt.command_id, f.commands.find(c => c.op === 'submit').command_id);
    assert.equal(f.commands.filter(c => c.op === 'submit').length, 1);
    f.workspace.close();
});
test('confirmed admission clears only the submitted draft', async () => {
    const f = fixture(), key = f.workspace.open('vessel', 'a', 'A'); await f.workspace.refresh(key);
    f.workspace.draft(key, 'Hello'); await f.workspace.act(key, 'submit');
    assert.equal(f.workspace.tabs.get(key)?.draft, ''); assert.equal(f.storage.length, 0);
    assert.equal(f.workspace.tabs.get(key)?.notice, '', 'successful admission uses the in-thread run status rather than a receipt banner');
    const command = f.commands.find(c => c.op === 'submit');
    assert.equal(command.expected_revision, 1); assert.equal(command.prompt, 'Hello'); assert.ok(command.command_id);
    f.workspace.close();
});
test('unknown-after-restart receipt remains blocking and never replays', async () => {
    const f = fixture(), key = f.workspace.open('vessel', 'a', 'A'); await f.workspace.refresh(key);
    f.workspace.draft(key, 'Hello'); f.mode('unknown'); await f.workspace.act(key, 'submit');
    f.mode('unknown_after_restart'); await f.workspace.reconcile(key);
    assert.equal(f.storage.length, 1); assert.equal(f.workspace.actionable(f.workspace.tabs.get(key)!), false);
    f.workspace.close();
});
test('stale snapshot cannot authorize commands', async () => {
    const f = fixture(), key = f.workspace.open('vessel', 'a', 'A'); await f.workspace.refresh(key);
    const tab = f.workspace.tabs.get(key)!; tab.freshAt = Date.now() - 36000; f.workspace.draft(key, 'Hello');
    await f.workspace.act(key, 'submit'); assert.equal(f.commands.filter(c => c.op === 'submit').length, 0);
    f.workspace.close();
});

test('expired and mismatched decisions never dispatch responses', async () => {
    const f=fixture(),key=f.workspace.open('vessel','a','A');await f.workspace.refresh(key);
    await f.workspace.respond(key,{decision_id:'decision',expires_at_ms:Date.now()-1,incarnation:'incarnation'},'approved');
    await f.workspace.respond(key,{decision_id:'decision',expires_at_ms:Date.now()+60000,incarnation:'other'},'approved');
    assert.equal(f.commands.filter(c=>c.op==='respond').length,0);f.workspace.close();
});
test('root consent remains a typed response fenced to exact run and expiry', async () => {
    const f=fixture(),key=f.workspace.open('vessel','a','A');await f.workspace.refresh(key);
    f.workspace.tabs.get(key)!.snapshot.run.run_id='run';
    const expiry=Date.now()+20000;
    await f.workspace.respond(key,{decision_id:'decision',expires_at_ms:expiry,incarnation:'incarnation',run_id:'run'},{root_grant:'approved'});
    const command=f.commands.find(c=>c.op==='respond');
    assert.deepEqual(command.response,{root_grant:'approved'});assert.equal(command.run_id,'run');assert.equal(command.incarnation,'incarnation');assert.equal(command.expires_at_ms,expiry);f.workspace.close();
});
test('pictures are bounded, private, and blocked as steering', async () => {
    const f=fixture(),key=f.workspace.open('vessel','a','A');await f.workspace.refresh(key);
    await f.workspace.attach(key,[new File(['picture'],'example.png',{type:'image/png'})]);
    assert.equal(f.workspace.tabs.get(key)!.pictures.length,1);assert.equal(f.storage.length,0);
    await f.workspace.act(key,'steer');assert.equal(f.commands.filter(c=>c.op==='steer').length,0);
    await f.workspace.attach(key,[new File(['unsafe'],'example.svg',{type:'image/svg+xml'})]);
    assert.equal(f.workspace.tabs.get(key)!.pictures.length,1);f.workspace.close();
});

test('history paging and expansion preserve canonical indices across refresh',async()=>{
 const f=fixture(),key=f.workspace.open('vessel','a','A');await f.workspace.refresh(key);
 const tab=f.workspace.tabs.get(key)!;tab.snapshot.message_offset=2;tab.snapshot.messages=[{message_index:2,role:'assistant',content:'preview',projection_truncated:true}];
 const original=f.connection.client.exchange.bind(f.connection.client);
 f.connection.client.exchange=async(payload:any)=>{const c=payload.command;if(c.op==='history')return {protocol:1,outcome_unknown:false,result:{session_id:'a',incarnation:'incarnation',result:{revision:1,message_offset:c.offset,next_offset:2,messages:[{message_index:0,role:'user',content:'first'},{message_index:1,role:'assistant',content:'second'}]}}};if(c.op==='message_chunk')return {protocol:1,outcome_unknown:false,result:{session_id:'a',incarnation:'incarnation',result:{data:JSON.stringify({role:'assistant',content:'complete'}),has_more:false,total_bytes:50,next_offset:50}}};return original(payload);};
 await f.workspace.earlier(key);assert.deepEqual(tab.snapshot.messages.map((m:any)=>m.message_index),[0,1,2]);await f.workspace.expand(key,2);assert.equal(tab.snapshot.messages[2].content,'complete');await f.workspace.refresh(key);assert.equal(tab.snapshot.message_offset,0);assert.equal(tab.snapshot.messages[2].content,'complete');f.workspace.close();
});
test('image submission uses promoted attachment and separate durable mutation receipt',async()=>{
 const f=fixture(),key=f.workspace.open('vessel','a','A');await f.workspace.refresh(key);
 const original=f.connection.client.exchange.bind(f.connection.client);const uploads:any[]=[];
 f.connection.client.exchange=async(payload:any)=>{if(payload.command.op==='upload_image'){uploads.push(payload.command);return {protocol:1,outcome_unknown:false,result:{session_id:'a',incarnation:'incarnation',result:{id:'artifact',sha256:'a'.repeat(64),byte_size:7,media_type:'image/png'}}};}return original(payload);};
 await f.workspace.attach(key,[new File(['picture'],'example.png',{type:'image/png'})]);await f.workspace.act(key,'submit');
 assert.equal(uploads.length,1);assert.ok(uploads[0].upload_id);const command=f.commands.find(c=>c.op==='submit_content');assert.equal(command.content[0].attachment.id,'artifact');assert.equal(f.workspace.tabs.get(key)!.pictures.length,0);assert.equal(f.storage.length,0);f.workspace.close();
});

test('history-only connection remains readable when decisions are forbidden',async()=>{
 const f=fixture();const original=f.connection.client.exchange.bind(f.connection.client);f.connection.client.exchange=async(payload:any)=>payload.command.op==='decisions'?{protocol:1,outcome_unknown:false,error:'forbidden'}:payload.command.op==='capabilities'?{protocol:1,outcome_unknown:false,result:{scope:'scoped',rights:['history']}}:original(payload);
 const key=f.workspace.open('vessel','a','A');await f.workspace.refresh(key);assert.equal(f.workspace.tabs.get(key)!.stale,false);assert.equal(f.workspace.tabs.get(key)!.snapshot.session_id,'a');f.workspace.close();
});
test('delayed output from an old run is rejected',async()=>{
 const f=fixture(),key=f.workspace.open('vessel','a','A');await f.workspace.refresh(key);const tab=f.workspace.tabs.get(key)!;tab.snapshot.run={run_id:'old',live_text:'OLD'};let release:any;
 const original=f.connection.client.exchange.bind(f.connection.client);f.connection.client.exchange=async(payload:any)=>payload.command.op==='run_output'?await new Promise(resolve=>{release=()=>resolve({protocol:1,outcome_unknown:false,result:{session_id:'a',incarnation:'incarnation',result:{run_id:'old',offset:3,next_offset:7,data:'PAGE',has_more:false}}});}):original(payload);
 const pending=f.workspace.output(key,3);tab.snapshot={...tab.snapshot,run:{run_id:'new',live_text:'NEW'}};release();await assert.rejects(pending,/Output identity changed/);f.workspace.close();
});

test('image upload refusal is explained and does not reserve a submit command',async()=>{
 const f=fixture(),key=f.workspace.open('vessel','a','A');await f.workspace.refresh(key);
 const original=f.connection.client.exchange.bind(f.connection.client);
 f.connection.client.exchange=async(payload:any)=>payload.command.op==='upload_image'
   ? {protocol:1,outcome_unknown:false,error:'invalid image header or resource limit'} : original(payload);
 await f.workspace.attach(key,[new File(['picture'],'example.png',{type:'image/png'})]);
 await f.workspace.act(key,'submit');
 const tab=f.workspace.tabs.get(key)!;
 assert.match(tab.notice,/invalid image header or resource limit/);
 assert.match(tab.notice,/no message was submitted/);
 assert.equal(tab.pictures.length,1);
 assert.equal(f.commands.some(c=>c.op==='submit_content'),false);
 assert.equal(f.storage.length,0);
 f.workspace.close();
});

test('uncertain image upload keeps its identity and requires status inspection before retry',async()=>{
 const f=fixture(),key=f.workspace.open('vessel','a','A');await f.workspace.refresh(key);
 let attempts=0;const original=f.connection.client.exchange.bind(f.connection.client);
 f.connection.client.exchange=async(payload:any)=>{
   if(payload.command.op==='upload_image'){attempts++;throw new Error('Reply timed out; command outcome may be unknown.');}
   return original(payload);
 };
 await f.workspace.attach(key,[new File(['picture'],'example.png',{type:'image/png'})]);
 await f.workspace.act(key,'submit');
 const tab=f.workspace.tabs.get(key)!;
 assert.match(tab.notice,/outcome may be unknown/);
 assert.equal(tab.pictures.length,1);assert.equal(attempts,1);
 assert.equal(f.commands.some(c=>c.op==='submit_content'),false);
 f.workspace.close();
});

test('two pictures wake Voyage and submit once from one Send after fresh status',async()=>{
 const f=fixture(),key=f.workspace.open('vessel','a','A');await f.workspace.refresh(key);
 const original=f.connection.client.exchange.bind(f.connection.client);
 const uploads:any[]=[];let incarnation='incarnation';
 f.connection.client.exchange=async(payload:any)=>{
   const c=payload.command;
   if(c.op==='upload_image'){
     uploads.push(c);incarnation='resumed';
     return {protocol:1,outcome_unknown:false,result:{session_id:'a',incarnation,result:{id:'artifact',sha256:'a'.repeat(64),byte_size:7,media_type:'image/png'}}};
   }
   if(c.op==='snapshot')return {protocol:1,outcome_unknown:false,result:{session_id:'a',incarnation,result:{session_id:'a',revision:1,observation_cursor:3,messages:[],run:{state:'idle'}}}};
   if(c.op==='decisions')return {protocol:1,outcome_unknown:false,result:{session_id:'a',incarnation,result:[]}};
   return original(payload);
 };
 // First read observed the old owner, upload is answered by its resumed owner.
 await f.workspace.refresh(key);
 await f.workspace.attach(key,[new File(['picture'],'2587.jpg',{type:'image/jpeg'}),new File(['picture'],'2588.jpg',{type:'image/jpeg'})]);
 await f.workspace.act(key,'submit');
 const tab=f.workspace.tabs.get(key)!;
 assert.equal(tab.notice,'');
 assert.equal(tab.incarnation,'resumed');assert.equal(tab.stale,false);
 assert.equal(uploads.length,2,'each picture uploaded only once');
 assert.equal(f.commands.filter(c=>c.op==='submit_content').length,1);
 assert.equal(tab.pictures.length,0);f.workspace.close();
});

test('post-upload changed revision or active run never silently submits',async()=>{
 for(const change of [{revision:2},{run:{state:'running'}}]){
  const f=fixture(),key=f.workspace.open('vessel','a','A');await f.workspace.refresh(key);
  let uploaded=false;const original=f.connection.client.exchange.bind(f.connection.client);
  f.connection.client.exchange=async(payload:any)=>{
   if(payload.command.op==='upload_image'){uploaded=true;return {protocol:1,outcome_unknown:false,result:{session_id:'a',incarnation:'resumed',result:{id:'artifact',sha256:'a'.repeat(64),byte_size:7}}};}
   if(uploaded&&payload.command.op==='snapshot')return {protocol:1,outcome_unknown:false,result:{session_id:'a',incarnation:'resumed',result:{session_id:'a',revision:1,run:{state:'idle'},messages:[],...change}}};
   return original(payload);
  };
  await f.workspace.attach(key,[new File(['picture'],'example.png',{type:'image/png'})]);await f.workspace.act(key,'submit');
  assert.equal(f.commands.some(c=>c.op==='submit_content'),false);
  assert.equal(f.storage.length,0);assert.equal(f.workspace.tabs.get(key)!.pictures.length,1);
  f.workspace.close();
 }
});

test('pictures during an active run queue once and send after observed completion',async()=>{
 const f=fixture(),key=f.workspace.open('vessel','a','A');await f.workspace.refresh(key);
 const tab=f.workspace.tabs.get(key)!;tab.snapshot.run={state:'running',run_id:'r'};
 await f.workspace.attach(key,[new File(['picture'],'photo.png',{type:'image/png'})]);
 f.workspace.draft(key,'Here?');await f.workspace.act(key,'submit');await f.workspace.act(key,'submit');
 assert.equal(tab.queuedPictureSend,true);assert.equal(f.commands.some(c=>c.op==='submit_content'),false);
 f.workspace.draft(key,'changed');assert.equal(tab.draft,'Here?');
 const original=f.connection.client.exchange.bind(f.connection.client);let uploads=0;
 f.connection.client.exchange=async(payload:any)=>{
  if(payload.command.op==='upload_image'){uploads++;return {protocol:1,outcome_unknown:false,result:{session_id:'a',incarnation:'incarnation',result:{id:'artifact',sha256:'a'.repeat(64),byte_size:7}}};}
  return original(payload);
 };
 await f.workspace.refresh(key);await new Promise(resolve=>setTimeout(resolve,10));
 assert.equal(uploads,1);assert.equal(f.commands.filter(c=>c.op==='submit_content').length,1);
 assert.equal(tab.queuedPictureSend,false);assert.equal(tab.pictures.length,0);assert.equal(tab.draft,'');
 await f.workspace.refresh(key);assert.equal(f.commands.filter(c=>c.op==='submit_content').length,1);f.workspace.close();
});

test('cancelling queued pictures preserves draft and prevents later auto-send',async()=>{
 const f=fixture(),key=f.workspace.open('vessel','a','A');await f.workspace.refresh(key);
 const tab=f.workspace.tabs.get(key)!;tab.snapshot.run={state:'running'};
 await f.workspace.attach(key,[new File(['picture'],'photo.png',{type:'image/png'})]);
 await f.workspace.act(key,'submit');f.workspace.cancelPictureSend(key);await f.workspace.refresh(key);
 assert.equal(tab.queuedPictureSend,false);assert.equal(tab.pictures.length,1);
 assert.equal(f.commands.some(c=>['submit_content','upload_image','steer'].includes(c.op)),false);f.workspace.close();
});
