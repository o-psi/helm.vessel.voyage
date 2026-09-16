import {request, uuid, voyageResult} from './vessel-client.js';
const copy = value => structuredClone(value);
export const imageTypes = ['image/png','image/jpeg','image/webp'];
export async function draftRequest(client, operation) {
    if (!client) throw new Error('Offline — edits retained on this device. Reconnect to save.');
    const response = await client.exchange(request('drafts',{operation}));
    if (response.protocol !== 1 || response.error != null || response.outcome_unknown !== false) throw new Error(response.error || 'Draft outcome uncertain. Retry reconciles the same operation; nothing is sent.');
    return response.result;
}
// Recovery is tenant + Vessel scoped. Execution identities never belong in shared documents.
export class SharedDraft {
    constructor({client, storage, key, changed = () => {}}) { Object.assign(this,{client,storage,key,changed}); this.record=null; this.document=null; this.pending=null; this.dirty=false; this.conflict=null; this.saving=null; }
    persist() { if(this.record) this.storage.setItem(this.key,JSON.stringify({record:this.record,document:this.document,pending:this.pending,dirty:this.dirty,deletion:this.deletion})); this.changed(this); }
    async open(record) {
        this.record=copy(record); this.document=copy(record.document); this.dirty=false; this.pending=null; this.conflict=null; this.deletion=null;
        try { const local=JSON.parse(this.storage.getItem(this.key)); if(local?.record?.draft_id===record.draft_id && (local.dirty || local.pending || local.deletion)) { Object.assign(this,local); if(record.revision!==local.record.revision && !local.pending) this.conflict=record; } } catch {}
        this.persist();
    }
    edit(parts) { this.document.parts=copy(parts); this.dirty=true; this.persist(); }
    async poll() {
        if(!this.record || this.saving) return;
        if(this.pending) return this.save();
        const remote=await draftRequest(this.client(),{op:'get',draft_id:this.record.draft_id});
        if(!remote || remote.revision!==this.record.revision) {
            if(this.dirty) {this.conflict=remote || {deleted:true}; this.changed(this);}
            else if(remote) await this.open(remote);
            else { this.record=null; this.document=null; this.storage.removeItem(this.key); this.changed(this); }
        }
        if(this.dirty && !this.conflict) await this.save();
    }
    async save() {
        if(this.saving) {await this.saving; return this.save();}
        if(this.conflict) throw new Error('Draft changed on another device. Choose the shared version or save your edits as a separate draft.');
        if(!this.dirty && !this.pending) return this.record;
        if(!this.pending) this.pending={op:'put',command_id:uuid(),draft_id:this.record.draft_id,expected_revision:this.record.revision,document:copy(this.document)};
        const operation=copy(this.pending); this.persist();
        this.saving=(async()=>{try {
            const saved=await draftRequest(this.client(),operation);
            this.record=saved; this.pending=null; this.dirty=JSON.stringify(this.document)!==JSON.stringify(operation.document); this.persist(); return saved;
        } catch(error) {
            // Never overwrite a remote revision to resolve a failed CAS.
            try {const remote=await draftRequest(this.client(),{op:'get',draft_id:operation.draft_id}); if(remote && remote.revision!==operation.expected_revision && JSON.stringify(remote.document)!==JSON.stringify(operation.document)) this.conflict=remote;} catch {}
            this.changed(this); throw error;
        } finally {this.saving=null;}})();
        return this.saving;
    }
    async fork() {
        const original=this.record.draft_id, document=copy(this.document), id=uuid();
        // Staged references are draft scoped. Copy verified bytes before publishing
        // the fork; never point a new draft at another draft's private storage.
        const images=[];
        for(const part of document.parts) if(part.type==='image') images.push([part,await imageBytes(this.client(),part.attachment,{draft_id:original})]);
        const created=await draftRequest(this.client(),{op:'put',command_id:uuid(),draft_id:id,expected_revision:0,document:{...document,parts:document.parts.filter(p=>p.type!=='image')}});
        for(const [part,blob] of images){const bytes=new Uint8Array(await blob.arrayBuffer());let binary='';for(let i=0;i<bytes.length;i+=8192)binary+=String.fromCharCode(...bytes.subarray(i,i+8192));part.attachment=await draftRequest(this.client(),{op:'upload_image',command_id:uuid(),draft_id:id,name:part.attachment.name,data_base64:btoa(binary)});}
        this.key=this.key.replace(original,id);this.record=created;this.document=document;this.pending=null;this.conflict=null;this.dirty=true;this.persist();return this.save();
    }
    async sent(record) {
        if(this.record?.draft_id!==record.draft_id || this.dirty || this.pending || this.record.revision!==record.revision) return false;
        this.deletion ||= {op:'put',command_id:uuid(),draft_id:record.draft_id,expected_revision:record.revision,document:{target:record.document.target,parts:[]}};this.persist();
        const cleared=await draftRequest(this.client(),this.deletion);
        this.deletion=null;this.record=cleared;
        if(this.dirty){this.persist();return false;}
        this.document=copy(cleared.document);this.persist();return true;
    }

}
export async function imageBytes(client, attachment, {draft_id,session_id}={}) {
    if(!imageTypes.includes(attachment.media_type) || attachment.byte_size>2*1024*1024) throw new Error('Unsupported image metadata.');
    const chunks=[]; let offset=0;
    while(offset<attachment.byte_size) {
        const value=draft_id ? await draftRequest(client,{op:'read_image',draft_id,attachment_id:attachment.id,offset,limit:65536}) : voyageResult(await client.exchange(request('read_artifact',{session_id,artifact_id:attachment.id,offset,limit:65536})),session_id).result;
        const bytes=Uint8Array.from(atob(value.data_base64),c=>c.charCodeAt(0));
        if(value.offset!==offset || !bytes.length || bytes.length>65536 || offset+bytes.length>attachment.byte_size) throw new Error('Invalid image chunk.');
        offset+=bytes.length; chunks.push(bytes);
    }
    const blob=new Blob(chunks,{type:attachment.media_type});
    const hash=Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',await blob.arrayBuffer())),v=>v.toString(16).padStart(2,'0')).join('');
    if(hash!==attachment.sha256) throw new Error('Image integrity check failed.');
    return blob;
}
export function updateText(parts,text) {
    const original=parts.filter(p=>p.type==='text').map(p=>p.text).join('');
    if(!parts.some(p=>p.type==='text'))return [...parts,{type:'text',text}];
    let start=0;while(start<original.length && start<text.length && original[start]===text[start])start++;
    let suffix=0;while(suffix<original.length-start && suffix<text.length-start && original[original.length-1-suffix]===text[text.length-1-suffix])suffix++;
    const end=original.length-suffix, insertion=text.slice(start,text.length-suffix);
    let offset=0,inserted=false;
    return parts.map(part=>{
        if(part.type!=='text')return part;
        const lo=offset,hi=offset+part.text.length;offset=hi;
        let value=part.text.slice(0,Math.max(0,Math.min(part.text.length,start-lo)));
        if(!inserted && (start<hi || hi===original.length)){value+=insertion;inserted=true;}
        value+=part.text.slice(Math.max(0,Math.min(part.text.length,end-lo)));
        return {type:'text',text:value};
    });
}
