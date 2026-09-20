import React, {useState} from 'react';
import type {Tab, Workspace} from './workspace';
export function Decisions({tab,workspace}: {tab:Tab;workspace:Workspace}) {
    return <section className="decisions" aria-label="Pending decisions">{tab.decisions.map(decision=><Decision key={decision.decision_id} decision={decision} tab={tab} workspace={workspace}/>)}</section>;
}
function Decision({decision,tab,workspace}: {decision:any;tab:Tab;workspace:Workspace}) {
    const [answer,setAnswer]=useState('');
    const valid=decision.expires_at_ms>Date.now()&&decision.incarnation===tab.incarnation&&decision.run_id===tab.snapshot?.run?.run_id;
    if(!valid)return null;
    const enabled=workspace.actionable(tab)&&workspace.permitted(tab,'respond'),request=decision.request;
    const respond=(response:unknown)=>workspace.respond(tab.key,decision,response);
    if(request?.kind==='approval')return <aside className="notice"><strong>Approval requested</strong><pre>{JSON.stringify(request.approval,null,2)}</pre><button disabled={!enabled} onClick={()=>void respond('approved')}>Approve</button><button disabled={!enabled} onClick={()=>void respond('denied')}>Deny</button></aside>;
    if(request?.kind==='question')return <aside className="notice"><strong>{request.question.question}</strong><p>Your answer becomes conversation history. Never enter a password or secret.</p><div className="decision-actions">{request.question.options.map((option:string,index:number)=><button key={index} disabled={!enabled} onClick={()=>void respond({status:'selected',index,answer:option})}>{option}</button>)}</div><label>Custom answer<input value={answer} onChange={event=>setAnswer(event.target.value)}/></label><button disabled={!enabled||!answer.trim()} onClick={()=>void respond({status:'custom',answer})}>Send custom answer</button><button disabled={!enabled} onClick={()=>void respond({status:'cancelled'})}>Skip question</button></aside>;
    if(request?.kind==='root_grant'){
        const grant=request.root_grant;
        const supported=grant&&typeof grant.path==='string'&&grant.path.startsWith('/')&&!/[\u0000-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/u.test(grant.path)&&['read','write'].includes(grant.permission)&&grant.lifetime==='current_run'&&typeof grant.reason==='string'&&grant.reason.trim();
        return <aside className="notice"><strong>Filesystem access requested</strong>{supported?<><dl><dt>Executing Vessel</dt><dd>{tab.vessel}</dd><dt>Directory</dt><dd>{grant.path}</dd><dt>Permission</dt><dd>{grant.permission==='write'?'Read and write':'Read only'}</dd><dt>Lifetime</dt><dd>Current run only</dd><dt>Reason</dt><dd>{grant.reason}</dd></dl><p>Not inherited by children or already-running processes.</p><button disabled={!enabled} onClick={()=>void respond({root_grant:'approved'})}>Grant access for this run</button><button disabled={!enabled} onClick={()=>void respond({root_grant:'denied'})}>Deny access</button></>:<p>Unsupported filesystem request. No access granted. Use an updated native Helm client.</p>}</aside>;
    }
    return <aside className="notice">This decision requires a native Helm client. No response sent.</aside>;
}
