import fs from 'node:fs/promises';
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {digest, refuse} from './security.mjs';
export const UUID=/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
export async function privateDir(dir) {
  await fs.mkdir(dir,{recursive:true,mode:0o700});
  const s=await fs.lstat(dir);
  if(!s.isDirectory()||s.isSymbolicLink()||(s.mode&0o077)||(process.getuid&&s.uid!==process.getuid())||await fs.realpath(dir)!==path.resolve(dir))refuse('unsafe_state_directory');
}
export class Journal {
  constructor(root){this.root=root;this.records=new Map();this.poisoned=false;}
  async init(){
    await privateDir(this.root);
    const files=await fs.readdir(this.root);if(files.length>10000)refuse('receipt_limit');
    for(const name of files){
      if(!name.endsWith('.json')||!UUID.test(name.slice(0,-5)))refuse('unsafe_receipt');
      const file=path.join(this.root,name),s=await fs.lstat(file);
      if(!s.isFile()||s.isSymbolicLink()||s.size>2048||(s.mode&0o077)||(process.getuid&&s.uid!==process.getuid()))refuse('unsafe_receipt');
      const r=JSON.parse(await fs.readFile(file,'utf8'));
      if(r.id!==name.slice(0,-5)||!['dispatched','unknown','completed','refused'].includes(r.state)||!/^[a-f0-9]{64}$/.test(r.digest))refuse('unsafe_receipt');
      if(r.state==='dispatched')r.state='unknown';this.records.set(r.id,r);
    }
  }
  async save(r){
    const dest=path.join(this.root,r.id+'.json'),tmp=dest+'.'+randomUUID();
    try {const f=await fs.open(tmp,'wx',0o600);try{await f.writeFile(JSON.stringify(r));await f.sync();}finally{await f.close();}
      await fs.rename(tmp,dest);const d=await fs.open(this.root,'r');try{await d.sync();}finally{await d.close();}this.records.set(r.id,r);
    }catch(e){this.poisoned=true;throw e;}
  }
  previous(req){const r=this.records.get(req.id);if(r&&r.digest!==digest(req))refuse('id_conflict');return r;}
  async begin(req){if(this.poisoned)refuse('journal_failed');if(this.records.size>=10000)refuse('receipt_limit');const r={id:req.id,digest:digest(req),state:'dispatched',time:Date.now()};await this.save(r);return r;}
  async finish(r,state,code){await this.save({...r,state,...(code?{code}:{})});}
}
