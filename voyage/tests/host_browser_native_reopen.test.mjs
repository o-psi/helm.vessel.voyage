import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,readFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {requestNativeReopen,validateNativeReopenReply,nativePageOwner} from './host_browser_native_reopen.mjs';
const client={label:'native-a',pid:901,start_ticks:500,descendants:true};
const request={id:'exact-id',digest:'a'.repeat(64),client};
const reply={schema:1,id:request.id,digest:request.digest,status:'observed',outcome_unknown:false,launcher:'/owned/fresh/open.html',client};
function fixturePage({fail=false,closed=false}={}){
 return {closed,calls:0,isClosed(){return this.closed;},async close(options){
  assert.deepEqual(options,{runBeforeUnload:false});this.calls++;
  if(fail)throw Error('fixed owned close failed');this.closed=true;
 }};
}
test('CUA Native ownership retains original A after active replacement and closes only its three created pages',async()=>{
 const unrelated=fixturePage(),created=[];
 const owner=nativePageOwner({async newPage(){const page=fixturePage();created.push(page);return page;},pages(){return [unrelated,...created];}});
 const native=[{page:await owner.open()},{page:await owner.open()}],oldA=native[0].page;
 native[0]={page:await owner.open()};assert.ok(native[0].page!==oldA);
 await assert.rejects(owner.open());
 assert.deepEqual(await owner.close(),{owned_pages:3,closed_pages:3,unresolved_pages:0});
 for(const page of created){assert.equal(page.calls,1);assert.equal(page.isClosed(),true);}
 assert.equal(unrelated.calls,0);assert.equal(unrelated.isClosed(),false);
 await assert.rejects(owner.open());
});
test('ownership is retained before setup and failure cleanup attempts every other owned page without false success',async()=>{
 const created=[fixturePage({closed:true}),fixturePage({fail:true}),fixturePage()],pending=[...created];
 const owner=nativePageOwner({async newPage(){return pending.shift();}});
 await owner.open();await owner.open();await owner.open();
 assert.deepEqual(await owner.close(),{owned_pages:3,closed_pages:2,unresolved_pages:1});
 assert.equal(created[0].calls,0);assert.equal(created[1].calls,1);assert.equal(created[2].calls,1);
});
test('fresh Native reply remains exact process bound and unknown/foreign/credential fields cannot pass',()=>{
 assert.equal(validateNativeReopenReply(reply,request),reply.launcher);
 for(const value of [{...reply,id:'other'},{...reply,digest:'b'.repeat(64)},{...reply,outcome_unknown:true},{...reply,status:'unknown'},
  {...reply,client:{...client,start_ticks:501}},{...reply,launcher:'https://private.example/'},{...reply,launcher:'/owned/credential.json'},{...reply,token:'not-accepted'}])assert.throws(()=>validateNativeReopenReply(value,request));
});
test('one private fixed fixture A request is issued and cannot repeat after observed response',{timeout:5000},async()=>{
 const directory=await mkdtemp(path.join(tmpdir(),'native-reopen-contract-'));
 const cfg={native_reopen_mailbox:directory,sessions:[{id:'fixture-a',label:'a'},{id:'fixture-b',label:'b'}],client_roots:[client,{...client,label:'native-b',pid:902}]};
 let poll;
 try{
  poll=setInterval(async()=>{try{const issued=JSON.parse(await readFile(path.join(directory,'request.json'),'utf8'));clearInterval(poll);
   await writeFile(path.join(directory,'response.json'),JSON.stringify({...reply,id:issued.id,digest:issued.digest}),{mode:0o600,flag:'wx'});}catch(error){if(error.code!=='ENOENT')clearInterval(poll);}},10);
  assert.equal(await requestNativeReopen(cfg),reply.launcher);
  const issued=JSON.parse(await readFile(path.join(directory,'request.json'),'utf8'));assert.equal(issued.action,'native_reopen');assert.equal(issued.label,'a');assert.equal(issued.session_id,'fixture-a');assert.deepEqual(issued.client,client);
  assert.deepEqual(Object.keys(issued).sort(),['action','client','digest','expires_at_ms','id','label','schema','session_id']);
  await assert.rejects(requestNativeReopen(cfg),error=>error.code==='EEXIST');
 }finally{clearInterval(poll);await rm(directory,{recursive:true,force:true});}
});
