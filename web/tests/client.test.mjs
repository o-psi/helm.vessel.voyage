import test from 'node:test';
import assert from 'node:assert/strict';
import {IntentJournal, mutation, request, resolved, voyageResult, VesselSocket} from '../resources/js/vessel-client.js';
const session = '10000000-0000-4000-8000-000000000001', incarnation = '10000000-0000-4000-8000-000000000002', run = '10000000-0000-4000-8000-000000000003';
const snapshot = {session_id:session,revision:12,run:{run_id:run}};
const storage = () => { const data = new Map(); return {get length(){return data.size;},key:i=>[...data.keys()][i],getItem:k=>data.get(k),setItem:(k,v)=>data.set(k,v),removeItem:k=>data.delete(k)}; };
const reply = result => ({protocol:1,error:null,outcome_unknown:false,result:{session_id:session,incarnation,result}});

test('mutations pin observed identities, fixed expiry and independent durable IDs', () => {
    const first = mutation('submit',snapshot,incarnation,{prompt:'hello'});
    assert.equal(first.command.expected_revision,12); assert.equal(first.command.session_id,session); assert.ok(!('incarnation' in first.command));
    const second = mutation('cancel',snapshot,incarnation);
    assert.equal(second.command.incarnation,incarnation); assert.equal(second.command.run_id,run); assert.notEqual(first.command.command_id,second.command.command_id);
    assert.ok(second.command.expires_at_ms > Date.now()); assert.throws(()=>mutation('submit',{revision:NaN},incarnation));
});
test('write-ahead journal records identities only, survives reload, isolates Vessel', () => {
    const store = storage(), journal = new IntentJournal(store,'vessel-a');
    const value = mutation('submit',snapshot,incarnation,{prompt:'not persisted'}).command;
    journal.prepare(value); const reload = new IntentJournal(store,'vessel-a');
    assert.equal(reload.entries()[0].command_id,value.command_id); assert.ok(!JSON.stringify(reload.entries()).includes('not persisted'));
    assert.equal(new IntentJournal(store,'vessel-b').entries().length,0); reload.settle(value.command_id); assert.equal(journal.entries().length,0);
});
test('storage refusal or corruption is not silently discarded', () => {
    const journal = new IntentJournal({getItem:()=>null,setItem:()=>{throw Error('quota');}},'a'); assert.throws(()=>journal.prepare(mutation('cancel',snapshot,incarnation).command));
    assert.throws(()=>new IntentJournal({length:1,key:()=> 'helm-web:intent:a:bad',getItem:()=>'{bad'},'a').entries());
});
test('receipt identity and status required; unknown never means safe to resend', () => {
    const id = mutation('cancel',snapshot,incarnation).command.command_id;
    assert.equal(resolved(reply({command_id:id,status:'unknown'}),id,session,true),false);
    assert.equal(resolved(reply({command_id:id,status:'accepted'}),id,session,true),true);
    assert.equal(resolved(reply({command_id:'different',status:'accepted'}),id,session,true),false);
    assert.equal(resolved({...reply({command_id:id,status:'accepted'}),outcome_unknown:true},id,session,true),false);
    assert.equal(resolved({...reply(null),error:'refused'},id,session),true);
    assert.equal(resolved({...reply(null),error:'refused'},id,session,true),false);
    assert.equal(resolved(reply({record:{request:{receipt_id:id}},status:'applied'}),id,session,true),true);
});
test('read identity checks refuse foreign sessions and changing incarnation', () => {
    assert.equal(voyageResult(reply(snapshot),session,incarnation).result.revision,12);
    assert.throws(()=>voyageResult(reply(snapshot),'other'));
    assert.throws(()=>voyageResult(reply(snapshot),session,'other'));
    assert.throws(()=>voyageResult({...reply(snapshot),outcome_unknown:true},session));
});
class Socket extends EventTarget {
    readyState = 1; frames = [];
    send(text) { this.frames.push(JSON.parse(text)); }
    close() { this.readyState=3; this.dispatchEvent(new Event('close')); }
    receive(value) { const event = new Event('message'); event.data = JSON.stringify(value); this.dispatchEvent(event); }
}
test('socket correlates replies and disconnect never replays mutations', async () => {
    const socket = new Socket(); let disconnected = 0;
    const client = new VesselSocket(socket,()=>disconnected++);
    const pending = client.exchange(request('catalogue')); const frame = socket.frames[0];
    socket.receive({type:'reply',request_id:'wrong',response:reply(null)});
    socket.receive({type:'reply',request_id:frame.request_id,response:reply([])}); assert.deepEqual(await pending,reply([]));
    const uncertain = client.exchange(mutation('submit',snapshot,incarnation,{prompt:'hello'}));
    socket.close(); await assert.rejects(uncertain,/outcome may be unknown/); assert.equal(disconnected,1); assert.equal(socket.frames.length,2);
});

test('independent tabs retain both write-ahead intents', () => {
    const store=storage(), a=new IntentJournal(store,'v'), b=new IntentJournal(store,'v');
    const first=mutation('submit',snapshot,incarnation,{prompt:'a'}).command;
    const second=mutation('submit',snapshot,incarnation,{prompt:'b'}).command;
    a.prepare(first); b.prepare(second); a.settle(first.command_id);
    assert.equal(b.entries().length,1);assert.equal(b.entries()[0].command_id,second.command_id);
});
