import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,readFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {requestNativeReopen,validateNativeReopenReply} from './host_browser_native_reopen.mjs';
const client={label:'native-a',pid:901,start_ticks:500,descendants:true};
const request={id:'exact-id',digest:'a'.repeat(64),client};
const reply={schema:1,id:request.id,digest:request.digest,status:'observed',outcome_unknown:false,launcher:'/owned/fresh/open.html',client};
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
