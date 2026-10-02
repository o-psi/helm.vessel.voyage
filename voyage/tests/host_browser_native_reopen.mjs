// One private request to the already owning Python TUI parent. Not a general
// command/credential interface; only fixture A's fixed viewer reopen is allowed.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {constants} from 'node:fs';
import path from 'node:path';
import {createHash,randomUUID} from 'node:crypto';
const hash=raw=>createHash('sha256').update(raw).digest('hex');
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const keys=['schema','id','digest','status','outcome_unknown','launcher','client'];
// Exactly two original Native pages and one reopened A. Claim each page as
// soon as this context creates it; replacing the active A never drops ownership.
// No ambient CUA tab, existing context page or caller-supplied handle is admitted.
export function nativePageOwner(context){
 const pages=new Set();let retired=false;
 return {
  async open(){
   assert.equal(retired,false);assert.ok(pages.size<3,'fixed Native page bound reached');
   const page=await context.newPage();pages.add(page);return page;
  },
  async close(){
   retired=true;let closed=0;
   for(const page of pages){
    if(!page.isClosed())await page.close({runBeforeUnload:false}).catch(()=>{});
    if(page.isClosed())closed++;
   }
   return {owned_pages:pages.size,closed_pages:closed,unresolved_pages:pages.size-closed};
  }
 };
}
export function validateNativeReopenReply(reply,request){
 assert.ok(reply&&typeof reply==='object');assert.deepEqual(Object.keys(reply).sort(),keys.toSorted());
 assert.equal(reply.schema,1);assert.equal(reply.id,request.id);assert.equal(reply.digest,request.digest);
 assert.equal(reply.status,'observed');assert.equal(reply.outcome_unknown,false);assert.deepEqual(reply.client,request.client);
 assert.ok(typeof reply.launcher==='string'&&path.isAbsolute(reply.launcher)&&path.basename(reply.launcher)==='open.html');
 return reply.launcher;
}
export async function requestNativeReopen(cfg){
 const directory=cfg.native_reopen_mailbox;
 assert.ok(path.isAbsolute(directory)&&cfg.sessions.length===2&&cfg.client_roots.length===2);
 const held=await fs.open(directory,constants.O_RDONLY|constants.O_DIRECTORY|constants.O_NOFOLLOW|constants.O_CLOEXEC);
 try{
  const pin=await held.stat();assert.ok(pin.isDirectory()&&pin.uid===process.getuid()&&!(pin.mode&0o077));
  const verify=async()=>{const current=await fs.lstat(directory);assert.ok(current.isDirectory()&&current.uid===process.getuid()&&!(current.mode&0o077)&&current.dev===pin.dev&&current.ino===pin.ino);};
  const item=cfg.sessions[0],request={schema:1,id:randomUUID(),action:'native_reopen',session_id:item.id,label:item.label,client:cfg.client_roots[0],expires_at_ms:Date.now()+45000};
  request.digest=hash(JSON.stringify(request));await verify();
  const fd=await fs.open(path.join(directory,'request.json'),constants.O_WRONLY|constants.O_CREAT|constants.O_EXCL|constants.O_NOFOLLOW|constants.O_CLOEXEC,0o600);
  try{await fd.writeFile(JSON.stringify(request));await fd.sync();}finally{await fd.close();}await held.sync();
  while(Date.now()<request.expires_at_ms){
   await verify();let replyFd;
   try{replyFd=await fs.open(path.join(directory,'response.json'),constants.O_RDONLY|constants.O_NOFOLLOW|constants.O_NONBLOCK|constants.O_CLOEXEC);}
   catch(error){if(error.code!=='ENOENT')throw error;await sleep(50);continue;}
   try{
    const meta=await replyFd.stat();assert.ok(meta.isFile()&&meta.uid===process.getuid()&&meta.nlink===1&&!(meta.mode&0o077)&&meta.size<=8192);
    const bytes=Buffer.alloc(meta.size+1),{bytesRead}=await replyFd.read(bytes,0,bytes.length,0),after=await replyFd.stat(),named=await fs.lstat(path.join(directory,'response.json'));
    assert.equal(bytesRead,meta.size);for(const key of ['dev','ino','size','mtimeMs','ctimeMs']){assert.equal(meta[key],after[key]);assert.equal(meta[key],named[key]);}
    await verify();return validateNativeReopenReply(JSON.parse(bytes.subarray(0,bytesRead)),request);
   }finally{await replyFd.close();}
  }
  throw Error('native reopen unconfirmed; no key action repeated');
 }finally{await held.close();}
}
