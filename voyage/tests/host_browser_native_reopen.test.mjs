import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,readFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {requestNativeReopen,validateNativeReopenReply,nativePageOwner} from './host_browser_native_reopen.mjs';
import {confirmPrivateReclaim,closeBrowser} from './host_browser_production.mjs';
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

function privateFixture({controls=true,foreign=false}={}){
 const old={attachment_id:'11111111-1111-4111-8111-111111111111',browser_id:'22222222-2222-4222-8222-222222222222',incarnation:'33333333-3333-4333-8333-333333333333',tab_id:'44444444-4444-4444-8444-444444444444',document_epoch:1,viewport_epoch:1,controller_epoch:2,capture_epoch:3};
 const fresh={...old,controller_epoch:3,capture_epoch:4};if(foreign)fresh.browser_id='55555555-5555-4555-8555-555555555555';
 const state={status:{running:true,mode:'private',binding:fresh,controller:controls?old.attachment_id:null},native_claims:new Map([['attach',{action:'attach',binding:{...old,attachment_id:'66666666-6666-4666-8666-666666666666'}}]])};
 const clicks=[];const page={locator(){return {async getAttribute(){return 'true';},async waitFor(){}};},getByRole(_role,{name}){return {async click(){clicks.push(name);assert.equal(name,'Browse privately');state.status.controller=old.attachment_id;}};},async waitForFunction(){assert.equal(state.status.mode,'private');}};
 return {old,state,page,clicks};
}
test('confirmed same-principal private Attach never toggles Finish private browsing',async()=>{
 const f=privateFixture();const proof=await confirmPrivateReclaim(f.page,f.state,f.old);
 assert.equal(proof.same_private_owner,true);assert.equal(proof.fresh_attach_request,true);assert.deepEqual(f.clicks,[]);
});
test('private observer uses one explicit reclaim and still proves exact owner and advanced fences',async()=>{
 const f=privateFixture({controls:false});const proof=await confirmPrivateReclaim(f.page,f.state,f.old);
 assert.equal(proof.fresh_control_and_capture_fences,true);assert.deepEqual(f.clicks,['Browse privately']);
});
test('foreign private browser is refused before any reclaim input',async()=>{
 const f=privateFixture({foreign:true});await assert.rejects(confirmPrivateReclaim(f.page,f.state,f.old));assert.deepEqual(f.clicks,[]);
});
test('final cleanup reacquires public control after idle detach before one Close',async()=>{
 let phase='agent';const clicks=[];const page={
  locator(selector){return {async getAttribute(){return selector==='.host-browser-viewer'?phase:'true';},async waitFor(){}};},
  getByLabel(name){return {async click(){clicks.push(name);}};},
  getByRole(_role,{name}){return {async click(){clicks.push(name);if(name==='Use browser')phase='human';else if(name==='Close browser'){assert.equal(phase,'human');phase='stopped';}else assert.fail('unexpected input');}};},
  async waitForFunction(_fn,wanted){assert.equal(phase,wanted||'stopped');},
 };
 await closeBrowser(page);assert.equal(phase,'stopped');assert.deepEqual(clicks,['More browser options','Use browser','More browser options','Close browser']);
});
