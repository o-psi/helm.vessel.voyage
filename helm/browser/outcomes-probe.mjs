// Focused gate/wire tests against the actual helper functions, without Chromium.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import vm from 'node:vm';
const source=await fs.readFile(new URL('./helper.mjs',import.meta.url),'utf8');
function section(start,end){assert.equal(source.split(start).length,2);return source.slice(source.indexOf(start),source.indexOf(end,source.indexOf(start)));}
const S={controller:{seen:Date.now()},prompts:new Map()};
let timer;
const events=[];
const sandbox={S,authority:()=>{},crypto:{randomUUID:()=> 'prompt'},limits:{prompt_timeout_ms:1},output:e=>events.push(e),setTimeout:fn=>(timer=fn,1),clearTimeout:()=>{},console};
vm.createContext(sandbox);
vm.runInContext(section('async function confirmation(', '\nasync function observe(')+section('function localReason(', '\nasync function dispatch('),sandbox);
const job={id:'stable',epoch:1,action_sha256:'a'.repeat(64)};
for(const reason of ['denied','expired','invalidated','cancelled','approved']){
  const pending=sandbox.confirmation(job,{});
  assert.equal(S.prompts.size,1);
  if(reason==='expired')timer();else S.prompts.get('prompt').resolve(reason);
  assert.equal(await pending,reason);
  assert.equal(S.prompts.size,0);
}
S.controller=null;
assert.equal(await sandbox.confirmation(job,{}),'unavailable');
for(const reason of ['denied','expired','invalidated','unavailable','cancelled']){
  for(const state of ['refused','unknown']){
    const result=sandbox.wireResult(job,{state,code:'local_confirmation_'+reason},null,{code:'withheld'});
    assert.equal(result.local_reason,reason);
    assert.equal(result.state,state==='unknown'?'unresolved':'refused');
    assert.equal(result.request_id,job.id);
    assert.equal(result.action_sha256,job.action_sha256);
    assert.equal(result.image,null);
  }
}
assert.equal(sandbox.localReason('request_expired'),'expired');
assert.equal(sandbox.localReason('authority_fenced'),'invalidated');
assert.equal(sandbox.localReason('operation_failed'),null);
assert.match(source,/if\(consent!=='approved'\)refuse\('local_confirmation_'\+consent\)/);
assert.match(source,/p.resolve\(reason==='remote_cancellation'\?'cancelled':'invalidated'\)/);
assert.match(source,/p.resolve\(b.allow===true\?'approved':'denied'\)/);
console.log('PASS: local consent matrix, bounded expiry, wire reasons and unknown-state precedence');
