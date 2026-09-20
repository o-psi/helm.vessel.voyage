import React,{useState} from 'react';
import {actionDescription,actionDuration,actionStatus} from '../js/tool-presentation.js';
export type ToolEntry={key:string;call?:any;request?:any;result?:any};
export type ThreadRow={key:string;message?:any;entries?:ToolEntry[]};
export function threadRows(messages:any[]):ThreadRow[]{
    const calls=new Set(messages.flatMap(message=>(message.tool_calls||[]).map((call:any)=>call.id)).filter(Boolean));
    const results=new Map(messages.filter(message=>['tool','function'].includes(message.role)&&message.tool_call_id).map(message=>[message.tool_call_id,message]));
    const rows:ThreadRow[]=[];let group:ThreadRow|undefined;
    messages.forEach((message,index)=>{
        if(message.role==='system')return;
        const key=String(message.message_index??index),result=['tool','function'].includes(message.role);
        if(result&&calls.has(message.tool_call_id))return;
        const text=message.parts?.some((part:any)=>part.type==='image'||(part.type==='text'&&part.text?.trim()))||String(message.content||'').trim();
        if(!result&&(!message.tool_calls?.length||text)){rows.push({key:`message:${key}`,message});group=undefined;}
        if(!result&&!message.tool_calls?.length)return;
        if(!group){group={key:`tools:${key}`,entries:[]};rows.push(group);}
        if(message.tool_calls?.length)message.tool_calls.forEach((call:any,i:number)=>group!.entries!.push({key:call.id||`${key}:${i}`,call,request:message,result:results.get(call.id)}));
        else group.entries!.push({key:`result:${key}`,result:message});
    });
    return rows;
}
function Entry({entry,running,messageStart,decisions,renderMessage}:{entry:ToolEntry;running:boolean;messageStart:number;decisions:boolean;renderMessage:(message:any)=>React.ReactNode}){
    const [open,setOpen]=useState(false);
    const active=running&&Number.isSafeInteger(messageStart)&&entry.request?.message_index>=messageStart;
    const status=actionStatus(entry.call||{},entry.result,active,decisions);
    const label=[status,actionDuration(entry.result),entry.call?actionDescription(entry.call):entry.result?.name||'Tool result'].filter(Boolean).join(' · ');
    const args=entry.call?.function?.arguments??entry.call?.arguments;
    return <details className="tool-entry" open={open} onToggle={event=>setOpen(event.currentTarget.open)} data-tool-id={entry.key}><summary>{label}</summary>{open&&<div className="tool-body">{args!=null&&<pre>{typeof args==='string'?args:JSON.stringify(args,null,2)}</pre>}{entry.request?.projection_truncated&&renderMessage({...entry.request,content:'',parts:[],tool_calls:[]})}{entry.result&&renderMessage(entry.result)}</div>}</details>;
}
export function ToolGroup({entries,running,messageStart,decisions,renderMessage}:{entries:ToolEntry[];running:boolean;messageStart:number;decisions:boolean;renderMessage:(message:any)=>React.ReactNode}){
    const [older,setOlder]=useState(false);const count=Math.max(0,entries.length-3);
    return <section className="tool-group" aria-label="Tool actions">{count>0&&<div className="tool-group-heading"><span>{count} older actions · {entries.slice(0,count).filter(entry=>entry.result).length}/{count} finished</span><button type="button" onClick={()=>setOlder(!older)}>{older?'Hide older actions':'Show older actions'}</button></div>}{entries.map((entry,index)=><div key={entry.key} hidden={!older&&index<count}><Entry entry={entry} running={running} messageStart={messageStart} decisions={decisions} renderMessage={renderMessage}/></div>)}</section>;
}
