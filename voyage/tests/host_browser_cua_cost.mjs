// Supported CUA CDP only. Call on the two qualification tabs, never an ambient
// profile/browser target. Events are reduced locally; no raw event is returned.
const methods=['Network.webSocketCreated','Network.webSocketClosed',
 'Network.webSocketFrameSent','Network.webSocketFrameReceived','Runtime.consoleAPICalled','Target.attachedToTarget','Target.detachedFromTarget'];
const metricNames=['TaskDuration','ScriptDuration','LayoutDuration','RecalcStyleDuration',
 'JSHeapUsedSize','JSHeapTotalSize','Nodes','Documents','JSEventListeners'];
const allowedLabel=value=>typeof value==='string'&&/^[A-Za-z0-9_.-]{1,48}$/.test(value);
const fail=()=>{throw Error('bounded qualification CDP observation unavailable');};
const tabIdentity=value=>{
 if(typeof value==='number')return Number.isSafeInteger(value)&&value>0?String(value):null;
 return typeof value==='string'&&/^[1-9][0-9]*$/.test(value)&&Number.isSafeInteger(Number(value))&&String(Number(value))===value?value:null;
};
const bytes=frame=>frame.opcode===2?atob(frame.payloadData).length:new TextEncoder().encode(frame.payloadData).length;

export async function cuaBrowserCost(tab,{label,socketUrl,tabId,connectionId,sessionIds=[]}){
 const selectedTab=tabIdentity(tab?.id);
 if(!allowedLabel(label)||!selectedTab||tabIdentity(tabId)!==selectedTab||!connectionId||sessionIds.length!==2)fail();
 const url=new URL(socketUrl);if(url.protocol!=='wss:'||url.pathname!=='/v1/vessel/browser-socket'||url.search||url.hash||url.username||url.password)fail();
 const cap=await tab.capabilities.get('cdp');
 // The capability defaults to this selected tab and current origin. Raw
 // Target.getTargetInfo is unsupported; never discover or attach ambient targets.
 await cap.send('Network.enable',{});
 await cap.send('Performance.enable',{});
 await cap.send('Runtime.enable',{});
 const seed=await cap.readEvents({limit:1,methods,timeoutMs:0});
 if(seed.truncated||!Number.isSafeInteger(seed.cursor))fail();
 let cursor=seed.cursor,closed=false,window=null;const expires=Date.now()+600000;
 const sockets=new Set(),effects=new Set(),requests=new Map(),children=new Set();
 const totals={sent_bytes:0,received_bytes:0,sent_frames:0,received_frames:0,connections:0,closed_connections:0,authenticated_acks:0,renewal_acks:0,
  duplicate_effects:0,unknown_effects:0,refused_effects:0};
 let observedTarget=null;
 const sourceMatches=source=>{
  if(!source||tabIdentity(source.tabId)!==selectedTab)return false;
  if(source.targetId!==undefined){
   if(typeof source.targetId!=='string'||!source.targetId)return false;
   if(observedTarget!==null&&source.targetId!==observedTarget)return false;
   observedTarget=source.targetId;
  }
  return true;
 };
 const recordCommand=text=>{
  if(!text.includes('"command"'))return;
  const frame=JSON.parse(text),command=frame.request?.command,op=command?.operation;
  if(frame.type!=='command'||command?.op!=='host_browser'||!sessionIds.includes(command.session_id)||!op?.command_id||['status','mirror','receipt'].includes(op.action))return;
  if(effects.has(op.command_id))totals.duplicate_effects++;
  effects.add(op.command_id);requests.set(frame.request_id,op.command_id);
  if(effects.size>256||requests.size>64)fail();
 };
 const recordReply=text=>{
  if(!text.includes('"reply"'))return;
  const frame=JSON.parse(text),id=requests.get(frame.request_id);if(frame.type!=='reply'||!id)return;
  requests.delete(frame.request_id);const outer=frame.response,inner=outer?.result?.result;
  if(outer?.outcome_unknown!==false||inner?.outcome_unknown===true)totals.unknown_effects++;
  else if(outer?.error!=null||inner?.error!=null)totals.refused_effects++;
  // A verified non-dispatch may repeat its exact ID with newly observed fences.
  if(inner?.status==='prepared'&&inner.not_dispatched===true)effects.delete(id);
 };
 async function poll(){
  if(closed||Date.now()>=expires)fail();
  for(let page=0;page<20;page++){
   const result=await cap.readEvents({afterSequence:cursor,limit:1000,methods,timeoutMs:0});
   if(result.truncated||!Number.isSafeInteger(result.cursor)||result.cursor<cursor||result.events.length>1000)fail();
   for(const event of result.events){
    if(!sourceMatches(event.source))fail();
    const p=event.params||{};
    if(event.method==='Target.attachedToTarget'){
     if(p.targetInfo?.type==='iframe'&&typeof p.sessionId==='string')children.add(p.sessionId);
     if(children.size>16)fail();continue;
    }
    if(event.method==='Target.detachedFromTarget'){children.delete(p.sessionId);continue;}
    if(event.method==='Runtime.consoleAPICalled'){
     if(p.args?.[0]?.value!=='[Helm connection]')continue;
     const props=p.args?.[1]?.preview?.properties||[];
     // Fixed production metadata; never fetch an objectId/console payload.
     const metadata=Object.fromEntries(props.filter(v=>['event','connection'].includes(v.name)).map(v=>[v.name,v.value]));
     if(metadata.connection===connectionId&&['connected','renewal_ack'].includes(metadata.event)){
      totals.authenticated_acks++;if(metadata.event==='renewal_ack')totals.renewal_acks++;
     }
     continue;
    }
    if(event.method==='Network.webSocketCreated'){
     if(p.url===socketUrl){sockets.add(p.requestId);totals.connections++;if(sockets.size>64)fail();}
     continue;
    }
    if(event.method==='Network.webSocketClosed'){if(sockets.delete(p.requestId))totals.closed_connections++;continue;}
    if(!sockets.has(p.requestId))continue;
    const frame=p.response;if(!frame||typeof frame.payloadData!=='string'||frame.payloadData.length>4*1024*1024)fail();
    // Application frames only: control frames and TLS/HTTP overhead are separate.
    if(![1,2].includes(frame.opcode))continue;
    const sent=event.method==='Network.webSocketFrameSent';
    totals[sent?'sent_bytes':'received_bytes']+=bytes(frame);totals[sent?'sent_frames':'received_frames']++;
    if(frame.opcode===1){if(sent)recordCommand(frame.payloadData);else recordReply(frame.payloadData);}
   }
   cursor=result.cursor;if(!result.hasMore)return;
  }
  fail();
 }
 async function metrics(){
  const response=await cap.send('Performance.getMetrics',{});
  const all=Object.fromEntries(response.metrics.map(m=>[m.name,m.value]));
  if(metricNames.some(name=>!Number.isFinite(all[name])))fail();
  return Object.fromEntries(metricNames.map(name=>[name,all[name]]));
 }
 const observer={
  poll,
  async begin(){
   await poll();if(sockets.size!==1||totals.authenticated_acks!==totals.connections||!totals.connections||window)fail();
   window={at:Date.now(),metrics:await metrics(),traffic:{...totals}};
  },
  async finish(){
   if(!window)fail();await poll();if(sockets.size!==1||totals.authenticated_acks!==totals.connections)fail();
   const after=await metrics(),before=window;window=null;
   if(after.TaskDuration<before.metrics.TaskDuration)fail();
   return {label,status:totals.duplicate_effects||totals.unknown_effects||totals.refused_effects||requests.size?'unknown':'observed',scope:'actual CUA qualification Web tab renderer and public WSS application payload',
    captured_at_ms:before.at,elapsed_ms:Date.now()-before.at,task_seconds:after.TaskDuration-before.metrics.TaskDuration,
    heap_used_bytes:after.JSHeapUsedSize,heap_delta_bytes:after.JSHeapUsedSize-before.metrics.JSHeapUsedSize,
    nodes:after.Nodes,node_delta:after.Nodes-before.metrics.Nodes,
    sent_bytes:totals.sent_bytes-before.traffic.sent_bytes,received_bytes:totals.received_bytes-before.traffic.received_bytes,
    sent_frames:totals.sent_frames-before.traffic.sent_frames,received_frames:totals.received_frames-before.traffic.received_frames,
    selected_connection_observed:true,renewal_acks:totals.renewal_acks,duplicate_effects:totals.duplicate_effects,
    unknown_effects:totals.unknown_effects,refused_effects:totals.refused_effects,
    pending_effects:requests.size,
    metric_scope:'selected target renderer metrics; renderer/process sharing is possible, so do not sum task/heap across tabs',
    child_targets_observed:children.size,truncated:false};
  },
  async stop(){closed=true;await cap.send('Performance.disable',{});await cap.send('Network.disable',{});}
 };
 observer.measureAt=async(startedAt,milliseconds=10000)=>{
  if(!Number.isSafeInteger(startedAt)||milliseconds!==10000||startedAt-Date.now()>30000||Date.now()-startedAt>1000)fail();
  // Drain the bounded event buffer while waiting and measuring. This sends no
  // page input and preserves the original scheduled metrics window and fences.
  while(Date.now()<startedAt){await poll();await new Promise(resolve=>setTimeout(resolve,Math.max(0,Math.min(250,startedAt-Date.now()))));}
  await observer.begin();const end=window.at+milliseconds;
  while(Date.now()<end){await new Promise(resolve=>setTimeout(resolve,Math.max(0,Math.min(250,end-Date.now()))));await poll();}
  return observer.finish();
 };
 return observer;
}
