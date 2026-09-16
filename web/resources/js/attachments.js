import {request,voyageResult} from './vessel-client.js';
export const imageTypes=['image/png','image/jpeg','image/webp'];
export async function imageBytes(client, attachment, {session_id}={}) {
    if(!imageTypes.includes(attachment.media_type) || attachment.byte_size>2*1024*1024) throw new Error('Unsupported image metadata.');
    const chunks=[]; let offset=0;
    while(offset<attachment.byte_size) {
        const value=voyageResult(await client.exchange(request('read_artifact',{session_id,artifact_id:attachment.id,offset,limit:65536})),session_id).result;
        const bytes=Uint8Array.from(atob(value.data_base64),c=>c.charCodeAt(0));
        if(value.offset!==offset || !bytes.length || bytes.length>65536 || offset+bytes.length>attachment.byte_size) throw new Error('Invalid image chunk.');
        offset+=bytes.length; chunks.push(bytes);
    }
    const blob=new Blob(chunks,{type:attachment.media_type});
    const hash=Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',await blob.arrayBuffer())),v=>v.toString(16).padStart(2,'0')).join('');
    if(hash!==attachment.sha256) throw new Error('Image integrity check failed.');
    return blob;
}
