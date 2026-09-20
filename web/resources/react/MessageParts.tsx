import React,{useEffect,useState,useRef} from 'react';
import type {Tab,Workspace} from './workspace';
export function ImagePart({attachment,tab,workspace}:{attachment:any;tab:Tab;workspace:Workspace}){
    const [url,setUrl]=useState(''),[notice,setNotice]=useState('');
    useEffect(()=>{let active=true,owned='';workspace.artifact(tab.key,attachment).then(blob=>{if(!active)return;owned=URL.createObjectURL(blob);setUrl(owned);}).catch(error=>active&&setNotice(error.message));return()=>{active=false;if(owned)URL.revokeObjectURL(owned);};},[attachment.id,attachment.sha256,tab.key]);
    return url?<img className="message-picture" src={url} alt={attachment.name||'Attached picture'}/>:<p>{notice||'Loading verified picture…'}</p>;
}
export function Output({tab,workspace}:{tab:Tab;workspace:Workspace}){
    const generation=useRef(0);
    const run=tab.snapshot?.run,initial=run?.stream_reconciled?run.live_text||'':run?.partial_text||'';
    const [extra,setExtra]=useState(''),[offset,setOffset]=useState(0),[more,setMore]=useState(false),[busy,setBusy]=useState(false),[notice,setNotice]=useState('');
    useEffect(()=>{generation.current++;setExtra('');setOffset((run?.stream_reconciled?run.live_text_offset||0:0)+new TextEncoder().encode(initial).length);setMore(Boolean(run?.stream_reconciled?run.live_text_truncated:run?.partial_text_truncated));},[run?.run_id,initial,run?.live_text_offset,run?.live_text_truncated,run?.partial_text_truncated]);
    if(!initial&&!more)return null;
    return <article className="live"><small>{run.stream_reconciled?'Live output · provisional':'Unreconciled output · may overlap history'}</small><pre>{initial}{extra}</pre>{more&&<button disabled={busy||!workspace.actionable(tab)} onClick={async()=>{const epoch=generation.current;setBusy(true);try{const page=await workspace.output(tab.key,offset);if(page&&epoch===generation.current){setExtra(value=>value+page.data);setOffset(page.next_offset);setMore(page.has_more);}}catch(error){setNotice(error instanceof Error?error.message:'Output unavailable.');}finally{setBusy(false);}}}>Load more output</button>}<p role="status">{notice}</p></article>;
}
