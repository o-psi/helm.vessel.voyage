import {SharedDraft,draftRequest,imageBytes,updateText,imageTypes} from './shared-drafts.js';
import {uuid} from './vessel-client.js';
export function draftComposer(root,{fleet,current,select,notice,changed}) {
    const prompt=root.querySelector('#prompt'), form=root.querySelector('#composer');
    const option=(label,value)=>{const node=document.createElement('option');node.textContent=label;node.value=value;return node;};
    const template=id=>root.querySelector(`#${id}`).content.firstElementChild.cloneNode(true);
    const panel=template('flux-draft-panel');
    prompt.prepend(panel);
    const status=form.querySelector('[data-draft-status]'), picker=form.querySelector('[data-draft-picker]'), images=panel.querySelector('[data-images]');
    const conflict=panel.querySelector('[data-draft-conflict]'), errors=panel.querySelector('[data-upload-errors]');
    const menu=form.querySelector('[data-draft-menu]');
    const closeMenu=()=>menu.hidePopover?.();
    const sessions=new Map(); let active=null, opening=0, listing=[], timer, uploading=false, rendered='', urls=[];
    const c=()=>fleet.connections.get(current().vessel);
    const text=(draft=active)=>draft?.document?.parts.filter(p=>p.type==='text').map(p=>p.text).join('') || '';
    function render(draft=active) {
        if(draft!==active) return;
        status.textContent=uploading?'Uploading…':draft?.conflict?'Not synced':draft?.pending?'Saving…':draft?.dirty?'Not saved yet':draft?.record?'Saved':'';
        status.title=draft?.record?'Saved drafts are available on your other devices connected to this Vessel':'';
        conflict.hidden=!draft?.conflict;
        errors.hidden=!errors.childElementCount;
        images.hidden=!(draft?.document?.parts || []).some(p=>p.type==='image');
        panel.hidden=images.hidden && conflict.hidden && errors.hidden;
        menu.querySelector('[data-discard]').disabled=!draft?.record || uploading;
        panel.querySelector('[data-shared]').hidden=!draft?.conflict || draft.conflict.deleted;
        panel.querySelector('[data-fork]').hidden=!draft?.conflict;
        const parts=draft?.document?.parts || [], fingerprint=JSON.stringify([draft?.record?.draft_id,parts.filter(p=>p.type==='image')]);
        if(rendered!==fingerprint) {
            rendered=fingerprint; urls.forEach(URL.revokeObjectURL);urls=[];images.replaceChildren();
            for(const [index,part] of parts.entries()) if(part.type==='image') {
                const card=template('flux-draft-image'), label=card.querySelector('[data-image-name]'), remove=card.querySelector('button'), img=card.querySelector('img');
                label.textContent=part.attachment.name; remove.type='button';remove.setAttribute('aria-label',`Remove ${part.attachment.name}`);img.alt=part.attachment.name;img.hidden=true;
                remove.onclick=()=>{draft.edit(draft.document.parts.filter(p=>p.type!=='image' || p.attachment.id!==part.attachment.id));schedule();};images.append(card);
                imageBytes(draft.client(),part.attachment,{draft_id:draft.record.draft_id}).then(blob=>{if(rendered!==fingerprint)return;const url=URL.createObjectURL(blob);urls.push(url);img.src=url;img.hidden=false;}).catch(()=>{label.textContent+= ' — preview unavailable; reconnect to retry';});
            }
        }
        for(const button of panel.querySelectorAll('button,input,select'))button.disabled=uploading;
        form.querySelector('#attach-picture').disabled=uploading;
        changed?.();
    }
    function instance(record) {
        const vessel=record._vessel, key=`helm-web:draft:${root.dataset.tenantId}:${fleet.connections.get(vessel)?.vessel_id}:${record.draft_id}`;
        if(!sessions.has(key)) sessions.set(key,new SharedDraft({client:()=>fleet.connections.get(vessel)?.client,storage:localStorage,key,changed:render}));
        return sessions.get(key);
    }
    async function open(record) {
        const vessel=current().vessel, mine=opening, draft=instance({...record,_vessel:vessel}); active=draft; const ready=draft.saving ? draft.saving : draft.open(record); prompt.value=text(draft); await ready; if(mine!==opening || active!==draft)return draft; render();
        errors.replaceChildren();render();
        for(let i=0;i<localStorage.length;i++){const key=localStorage.key(i);if(!key?.startsWith(`${draft.key}:upload:`))continue;const operation=JSON.parse(localStorage.getItem(key));const retry=template('flux-draft-retry');retry.dataset.uploadRetry='';retry.textContent=`Resume ${operation.name} upload`;retry.onclick=async()=>{try{const attachment=await draftRequest(draft.client(),operation);if(!draft.document.parts.some(p=>p.type==='image' && p.attachment.id===attachment.id))draft.edit([...draft.document.parts,{type:'image',attachment}]);await draft.save();localStorage.removeItem(key);retry.remove();render();}catch(error){notice(error.message);}};errors.append(retry);errors.hidden=false;panel.hidden=false;}
        return draft;
    }
    async function discover() {
        if(!c()?.client) return;
        const vessel=current().vessel, mine=opening;
        const result=await draftRequest(c().client,{op:'list'});if(!Array.isArray(result?.drafts))throw new Error('This Vessel does not support shared drafts. Update it before sending.'); if(current().vessel!==vessel || mine!==opening)return;
        listing=result.drafts;
        const prefix=`helm-web:draft:${root.dataset.tenantId}:${c()?.vessel_id}:`;
        for(let i=0;i<localStorage.length;i++){const key=localStorage.key(i);if(!key?.startsWith(prefix))continue;try{const recovery=JSON.parse(localStorage.getItem(key));if(recovery?.record && (recovery.dirty || recovery.pending || recovery.deletion) && !listing.some(r=>r.draft_id===recovery.record.draft_id))listing.push({...recovery.record,document:recovery.document});}catch{}}

        picker.replaceChildren(option('Choose a draft…',''),...listing.map(r=>option(`${r.document.target.type} · ${r.document.parts.filter(p=>p.type==='text').map(p=>p.text).join(' ').slice(0,50) || 'Pictures / empty draft'}`,r.draft_id)));
        picker.value=active?.record?.draft_id || '';
    }
    async function ensure(target=null) {
        if(active?.record)return active;
        const now=current(); target ||= now.session_id ? (now.running?{type:'steer',session_id:now.session_id,run_id:now.run_id,incarnation:now.incarnation}:{type:'message',session_id:now.session_id}) : null;
        if(!target) throw new Error('Choose a Vessel and workspace with New voyage before composing a new-chat draft.');
        const value=prompt.value;return open({draft_id:uuid(),revision:0,document:{target,parts:[{type:'text',text:value}]}});
    }
    function schedule(draft=active) {clearTimeout(timer);timer=setTimeout(()=>draft?.save().catch(e=>{if(active===draft)status.textContent=e.message;}),600);}
    prompt.addEventListener('input',async()=>{const value=prompt.value, mine=opening;try{const pending=ensure(), draft=active;if(draft)draft.edit(updateText(draft.document.parts,value));await pending;if(mine===opening && active===draft)schedule(draft);}catch(e){notice(e.message);}});
    picker.onchange=async()=>{closeMenu();const record=listing.find(r=>r.draft_id===picker.value);if(!record)return;const target=record.document.target;if(target.type==='new_chat')select(current().vessel,null,'New-chat draft',true);else if(target.session_id!==current().session_id)select(current().vessel,target.session_id,'Draft voyage',true);++opening;await open(record);};
    menu.querySelector('[data-new]').onclick=async()=>{closeMenu();clearTimeout(timer);const previous=active;if(previous?.dirty)previous.save().catch(e=>notice(e.message));++opening;active=null;prompt.value='';try{await ensure();render();}catch(e){notice(e.message);}};
    menu.querySelector('[data-discard]').onclick=async()=>{closeMenu();
        const draft=active;if(!draft?.record || uploading)return;
        if(!window.confirm('Discard this shared draft on all devices?'))return;
        clearTimeout(timer);
        const key=`${draft.key}:discard`;
        try{
            if(draft.saving)await draft.saving;
            let operation=JSON.parse(localStorage.getItem(key));
            if(!operation){operation={op:'delete',command_id:uuid(),draft_id:draft.record.draft_id,expected_revision:draft.record.revision};localStorage.setItem(key,JSON.stringify(operation));}
            uploading=true;render();
            await draftRequest(draft.client(),operation);
            draft.storage.removeItem(draft.key);localStorage.removeItem(key);sessions.delete(draft.key);
            if(active===draft){active=null;prompt.value='';}await discover();
        }catch(error){notice(`Draft retained until discard is confirmed: ${error.message}`);}
        finally{uploading=false;render();}
    };
    panel.querySelector('[data-shared]').onclick=async()=>{const draft=active, mine=++opening, remote=draft.conflict;draft.storage.removeItem(draft.key);await draft.open(remote);if(mine===opening && draft===active){prompt.value=text();render();}};
    panel.querySelector('[data-fork]').onclick=async()=>{const draft=active, mine=opening, original=JSON.stringify(draft.document);try{const fork=await draft.fork();sessions.set(fork.key,fork);if(mine===opening && active===draft && JSON.stringify(draft.document)===original){active=fork;prompt.value=text();render();}await discover();}catch(e){notice(e.message);}};
    const input=form.querySelector('#picture-files');
    form.querySelector('#attach-picture').onclick=()=>input.click();
    async function upload(files) {
        if(uploading)return notice('Wait for the current picture upload.');
        uploading=true;render();
        try {
            const draft=await ensure(); await draft.save();
            for(const file of files) {
                const attachments=draft.document.parts.filter(p=>p.type==='image');
                if(!imageTypes.includes(file.type) || !file.size || file.size>2097152 || attachments.length>=4 || attachments.reduce((sum,p)=>sum+p.attachment.byte_size,0)+file.size>2097152) throw new Error('Use PNG, JPEG or WebP; at most four pictures and 2 MiB total. Resize pictures before retrying.');
                const bytes=new Uint8Array(await file.arrayBuffer());let binary='';for(let i=0;i<bytes.length;i+=8192)binary+=String.fromCharCode(...bytes.subarray(i,i+8192));
                const operation={op:'upload_image',command_id:uuid(),draft_id:draft.record.draft_id,name:file.name || 'pasted-image',data_base64:btoa(binary)};
                const uploadKey=`${draft.key}:upload:${operation.command_id}`;localStorage.setItem(uploadKey,JSON.stringify(operation));
                const send=async()=>{const attachment=await draftRequest(draft.client(),operation);if(!draft.document.parts.some(p=>p.type==='image' && p.attachment.id===attachment.id))draft.edit([...draft.document.parts,{type:'image',attachment}]);await draft.save();localStorage.removeItem(uploadKey);};
                try{await send();}catch(error){const retry=template('flux-draft-retry');retry.type='button';retry.textContent=`Retry ${file.name || 'picture'} upload`;retry.onclick=()=>send().then(()=>{retry.remove();render();}).catch(e=>notice(e.message));if(active===draft){errors.append(retry);errors.hidden=false;panel.hidden=false;}throw error;}
            }
            await discover();
        } catch(error){notice(`Picture not confirmed: ${error.message} Text and existing pictures retained.`);}finally{uploading=false;input.value='';render();}
    }
    input.onchange=()=>upload([...input.files]);
    form.addEventListener('paste',event=>{const files=[...(event.clipboardData?.files || [])];if(files.length){event.preventDefault();upload(files);}});
    form.addEventListener('dragover',event=>{if(event.dataTransfer?.types.includes('Files'))event.preventDefault();});
    form.addEventListener('drop',event=>{if(event.dataTransfer?.files.length){event.preventDefault();upload([...event.dataTransfer.files]);}});
    const poll=setInterval(()=>{if(!root.isConnected){clearInterval(poll);urls.forEach(URL.revokeObjectURL);return;} discover().catch(()=>{});const draft=active;draft?.poll().then(()=>{if(active===draft && !draft.dirty && document.activeElement!==prompt)prompt.value=text(draft);}).catch(e=>{status.textContent='Not synced';status.title=e.message;});},3000);
    return {
        get active(){return active;},
        capture(){return {opening,vessel:current().vessel,session_id:current().session_id,key:active?.key,workspace:active?.document?.target?.workspace,target:active?.document?.target ? structuredClone(active.document.target) : null};},
        async select(){clearTimeout(timer);const previous=active;if(previous?.dirty && !previous.conflict)previous.save().catch(()=>{});const mine=++opening;active=null;rendered='';await discover();if(mine!==opening)return;const now=current();const record=now.session_id && (listing.find(r=>r.document.target.session_id===now.session_id && r.document.parts.some(p=>p.type==='image' || p.text?.length)) || listing.find(r=>r.document.target.session_id===now.session_id));if(record)await open(record);else render();},
        async newChat(vessel,workspace){select(vessel,null,'New-chat draft',true);++opening;active=null;prompt.value='';const draft=await ensure({type:'new_chat',workspace});await draft.save();await discover();},
        async created(vessel,session_id,workspace,origin){if(origin && (origin.opening!==opening || origin.vessel!==current().vessel || origin.session_id!==current().session_id || origin.key!==active?.key))return true;if(active?.document?.target.type!=='new_chat' || current().vessel!==vessel)return false;if(active.document.target.workspace!==workspace)throw new Error('Created voyage workspace differs from the draft; draft retained.');select(vessel,session_id,'New voyage',true);prompt.value=text();return true;},
        async restoreNew(session_id){if(active?.document?.target.type==='new_chat'){const draft=active;await draft.save();return {draft,record:structuredClone(draft.record),session_id};}return null;},
        async beforeSend(){
            if(uploading)throw new Error('Wait for picture validation before sending.');
            const now={...current()}, mine=opening, value=prompt.value, ready=ensure(), draft=active;
            if(draft && value!==text(draft))draft.edit(updateText(draft.document.parts,value));
            await ready;
            const valid=()=>{if(mine!==opening || draft!==active || now.vessel!==current().vessel || now.session_id!==current().session_id)throw new Error('Composer selection changed; nothing sent.');};
            valid();
            do{await draft.save();valid();}while(draft.dirty);
            const target=draft.document.target;
            if(target.type!=='new_chat' && target.session_id!==now.session_id)throw new Error('Draft belongs to another voyage.');
            if(target.type==='steer' && (target.run_id!==now.run_id || target.incarnation!==now.incarnation || !now.running))throw new Error('This steering draft targets a different or finished run. Keep it, or explicitly create a new message draft.');
            if(target.type==='message' && now.running)throw new Error('This is a message draft, not steering. Wait for this run to finish or explicitly create a steering draft.');
            if(now.running && draft.document.parts.some(p=>p.type==='image'))throw new Error('Pictures cannot steer an active run. Wait for it to finish; your draft is retained.');
            return {draft,record:structuredClone(draft.record),vessel:now.vessel};
        },
        async admitted(sent){
            if(!sent.draft){const connection=fleet.connections.get(sent.vessel);const key=`helm-web:draft:${root.dataset.tenantId}:${connection?.vessel_id}:${sent.record.draft_id}`;sent.draft=sessions.get(key);if(!sent.draft){sent.draft=new SharedDraft({client:()=>connection?.client,storage:localStorage,key});sessions.set(key,sent.draft);await sent.draft.open(sent.record);}}
            if(await sent.draft.sent(sent.record)){if(active===sent.draft || active?.key===sent.draft.key && !active.dirty && !active.pending && active.record?.revision===sent.record.revision){active=null;prompt.value='';render();}}await discover();},
        async promote(sent,session_id){
            const key=`${sent.draft.key}:promotion:${sent.record.revision}:${session_id}`;
            let operation=JSON.parse(localStorage.getItem(key));
            if(!operation){operation={op:'promote',command_id:uuid(),draft_id:sent.record.draft_id,expected_revision:sent.record.revision,session_id};localStorage.setItem(key,JSON.stringify(operation));}
            return (await draftRequest(sent.draft.client(),operation)).parts;
        },
    };
}
