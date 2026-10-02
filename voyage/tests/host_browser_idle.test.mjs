import test from 'node:test';import assert from 'node:assert/strict';
import {idleWindowQualified,idleReconnectQualified} from './host_browser_idle.mjs';
const proof={schema:1,phase:'idle',label:'a',browser_running:true,zero_viewers_observed:true,recorder_stop_ack_observed:true,worker_identity_unchanged:true,private_metadata_identity_unchanged:true,slot_retained:true,browser_root_identity_unchanged:true,started_at_ms:1000,ended_at_ms:11000,samples:40,scope:'private metadata'};
test('idle requires every independent producer and retained-resource observation',()=>{
 assert.equal(idleWindowQualified(proof,{label:'a'}).observed,true);
 for(const key of ['browser_running','zero_viewers_observed','recorder_stop_ack_observed','worker_identity_unchanged','private_metadata_identity_unchanged','slot_retained','browser_root_identity_unchanged'])assert.throws(()=>idleWindowQualified({...proof,[key]:false},{label:'a'}));
 assert.throws(()=>idleWindowQualified({...proof,samples:0},{label:'a'}));assert.throws(()=>idleWindowQualified({...proof,ended_at_ms:2000},{label:'a'}));assert.throws(()=>idleWindowQualified(proof,{label:'other'}));
});
test('normal reconnect preserves browser and requires a new attach without start or close',()=>{
 const prior={browser_id:'b',incarnation:'i',attachment_id:'old'};const current={running:true,binding:{...prior,attachment_id:'new'}};
 const effects={browser_starts:1,browser_closes:0};const claims=[{action:'status'},{action:'attach',binding:current.binding}];assert.equal(idleReconnectQualified(prior,current,claims,effects,effects).fresh_attach,true);
 assert.throws(()=>idleReconnectQualified(prior,{...current,binding:{...current.binding,browser_id:'replacement'}},claims,effects,effects));
 assert.throws(()=>idleReconnectQualified(prior,current,claims,effects,{...effects,browser_starts:2}));assert.throws(()=>idleReconnectQualified(prior,current,[],effects,effects));
});
