// Pure metadata contracts, no client/browser/native/executor effects.
import test from 'node:test';import assert from 'node:assert/strict';import {randomUUID} from 'node:crypto';
import {measurementWindowAligned,privateBindingSnapshot,privateReclaimQualified} from './host_browser_qualification_windows.mjs';
function binding(){return {attachment_id:randomUUID(),browser_id:randomUUID(),incarnation:randomUUID(),tab_id:randomUUID(),document_epoch:2,viewport_epoch:3,controller_epoch:4,capture_epoch:5};}
test('all real sampler intervals required; delayed short absent or wrong endpoints stay incomplete',()=>{
 assert.equal(measurementWindowAligned(10000,[{start:10010,end:20010},{start:10100,end:20100}]),true);
 for(const windows of [[],[{start:10000,end:NaN}],[{start:11001,end:21001}],[{start:10000,end:19000}],[{start:10000,end:20000},{start:10001,end:25000}]])assert.equal(measurementWindowAligned(10000,windows),false);
});
test('original private binding is a valid immutable pre-detach copy; nil baseline cannot pass',()=>{
 const before=binding(),status={mode:'private',binding:before,controller:before.attachment_id};const held=privateBindingSnapshot(status);
 status.binding={...before,attachment_id:'00000000-0000-0000-0000-000000000000'};assert.equal(held.attachment_id,before.attachment_id);assert.throws(()=>privateBindingSnapshot(status));
});
test('same actual private owner reattaches only with fresh request and advanced capture/control fences',()=>{
 const before=binding(),after={...before,controller_epoch:before.controller_epoch+1,capture_epoch:before.capture_epoch+1};
 const claims=new Map([['fresh',{action:'attach',command_id:randomUUID(),binding:{...after,attachment_id:randomUUID()}}]]);
 assert.equal(privateReclaimQualified(before,{mode:'private',binding:after,controller:before.attachment_id},claims).same_private_owner,true);
 for(const next of [{...after,attachment_id:randomUUID()},{...after,controller_epoch:before.controller_epoch},{...after,capture_epoch:before.capture_epoch}])assert.throws(()=>privateReclaimQualified(before,{mode:'private',binding:next,controller:next.attachment_id},claims));
 assert.throws(()=>privateReclaimQualified(before,{mode:'private',binding:after,controller:before.attachment_id},new Map()));
});
