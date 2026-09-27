import {request,uuid,voyageResult} from './vessel-client.js';
import {imageTypes} from './attachments.js';

// Unsent text and pictures live only for this page lifetime. No sync or persistence.
export function composer(root,{fleet,current,select,notice,changed}) {
    const prompt=root.querySelector('#prompt'), form=root.querySelector('#composer');
    const panel=root.querySelector('#flux-attachment-panel').content.firstElementChild.cloneNode(true);
    prompt.prepend(panel);
    const images=panel.querySelector('[data-images]'), input=form.querySelector('#picture-files');
    const entries=new Map(); let active=null, opening=0, loading=false;
    const text=()=>prompt.value;
    function render(){
        images.replaceChildren();
        for(const part of active?.document.parts || []) if(part.type==='image') {
            const card=root.querySelector('#flux-attachment-image').content.firstElementChild.cloneNode(true);
            const img=card.querySelector('img'); img.src=part.url; img.alt=part.name; img.hidden=false;
            card.querySelector('[data-image-name]').textContent=part.name;
            card.querySelector('button').onclick=()=>{URL.revokeObjectURL(part.url);active.document.parts=active.document.parts.filter(p=>p!==part);render();};
            images.append(card);
        }
        panel.hidden=!images.childElementCount; images.hidden=panel.hidden; changed?.();
    }
    function remember(){if(active?.document.target.type==='new_chat')entries.set('new',active);if(active)active.document.parts=[{type:'text',text:text()},...active.document.parts.filter(p=>p.type==='image')];}
    function ensure(target){
        if(active)return active;
        const now=current();
        active={key:uuid(),vessel:now.vessel,document:{target:target || {type:'message',session_id:now.session_id},parts:[{type:'text',text:text()}]}};
        if(now.session_id)entries.set(`${now.vessel}:${now.session_id}`,active);
        return active;
    }
    prompt.addEventListener('input',()=>{ensure();remember();});
    async function attach(files, guard){
        if(loading||guard&&!guard())return false; loading=true;let success=true;
        const entry=ensure();
        try {for(const file of files){
            const pictures=entry.document.parts.filter(p=>p.type==='image');
            if(!imageTypes.includes(file.type)||!file.size||pictures.length>=4||pictures.reduce((n,p)=>n+p.size,0)+file.size>2097152)throw new Error('Use PNG, JPEG or WebP; at most four pictures and 2 MiB total.');
            const bytes=new Uint8Array(await file.arrayBuffer());if(guard&&!guard())throw Error('Voyage changed; capture not attached'); let binary='';
            for(let i=0;i<bytes.length;i+=8192)binary+=String.fromCharCode(...bytes.subarray(i,i+8192));
            entry.document.parts.push({type:'image',name:file.name||'pasted-image',size:file.size,data_base64:btoa(binary),url:URL.createObjectURL(file),uploads:new Map()});
        }}catch(error){success=false;notice(error.message);}finally{loading=false;input.value='';render();}return success;
    }
    form.querySelector('#attach-picture').onclick=()=>input.click();
    input.onchange=()=>attach([...input.files]);
    form.addEventListener('paste',event=>{const files=[...(event.clipboardData?.files||[])];if(files.length){event.preventDefault();attach(files);}});
    form.addEventListener('dragover',event=>{if(event.dataTransfer?.types.includes('Files'))event.preventDefault();});
    form.addEventListener('drop',event=>{if(event.dataTransfer?.files.length){event.preventDefault();attach([...event.dataTransfer.files]);}});
    return {
        get active(){return active;},
        get newDraft(){return active?.document.target.type==='new_chat'?active:entries.get('new');},
        remember,
        dispose(){
            for(const entry of new Set([...entries.values(),active])) for(const part of entry?.document.parts||[]) if(part.type==='image')URL.revokeObjectURL(part.url);
            entries.clear();active=null;
        },
        capture(){return active?{opening,vessel:active.vessel,session_id:current().session_id,key:active.key,workspace:active.document.target.workspace,target:structuredClone(active.document.target)}:null;},
        async select(){opening++;const now=current();active=entries.get(`${now.vessel}:${now.session_id}`)||null;prompt.value=active?.document.parts.filter(p=>p.type==='text').map(p=>p.text).join('')||'';render();},
        async newChat(vessel,workspace){remember();const retained=active?.document.target.type==='new_chat'?active:entries.get('new');select(vessel,null,'New voyage',true);opening++;active=retained;if(active){active.vessel=vessel;active.document.target.workspace=workspace;prompt.value=active.document.parts.filter(p=>p.type==='text').map(p=>p.text).join('');}else{prompt.value='';ensure({type:'new_chat',workspace});}render();},
        async created(vessel,session_id,workspace,origin){if(!active||origin?.key!==active.key||origin.opening!==opening||vessel!==current().vessel)return false;if(active.document.target.workspace!==workspace)throw new Error('Created chat workspace differs from the composer.');select(vessel,session_id,'New voyage',true);entries.delete('new');active.document.target={type:'message',session_id};entries.set(`${vessel}:${session_id}`,active);return true;},
        attach,
        async beforeSend(){if(loading)throw new Error('Wait for picture preparation.');const entry=ensure();remember();if(current().running&&entry.document.parts.some(p=>p.type==='image'))throw new Error('Pictures cannot be sent as steering. Wait for this run to finish.');return {draft:entry,record:{document:{parts:[...entry.document.parts]}},text:text(),vessel:entry.vessel};},
        async promote(sent,session_id){
            const client=fleet.connections.get(sent.vessel)?.client;const content=[];
            for(const part of sent.record.document.parts){
                if(part.type==='text'){content.push(part);continue;}
                // Reuse uncertain upload identity for this picture and destination.
                let upload=part.uploads.get(session_id);if(!upload){upload={id:uuid()};part.uploads.set(session_id,upload);}
                if(!upload.attachment)upload.attachment=voyageResult(await client.exchange(request('upload_image',{session_id,upload_id:upload.id,name:part.name,data_base64:part.data_base64})),session_id).result;
                content.push({type:'image',attachment:upload.attachment});
            }
            return content;
        },
        async admitted(sent){
            const entry=sent.draft;if(!entry)return;
            const sameText=(entry===active?text():entry.document.parts.filter(p=>p.type==='text').map(p=>p.text).join(''))===sent.text;
            const sentImages=sent.record.document.parts.filter(p=>p.type==='image');
            if(!sameText)return;
            entry.document.parts=entry.document.parts.filter(p=>p.type==='image'&&!sentImages.includes(p));
            for(const part of sentImages)URL.revokeObjectURL(part.url);
            if(entry===active){prompt.value='';render();}
        },
    };
}
