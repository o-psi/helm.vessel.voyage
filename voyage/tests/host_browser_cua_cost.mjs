// Supported CUA CDP only. Call on the two qualification tabs, never an ambient
// profile/browser target. Events are reduced locally; no raw event is returned.
const methods=['Network.webSocketCreated','Network.webSocketClosed',
 'Network.webSocketFrameError','Network.webSocketFrameSent','Network.webSocketFrameReceived','Runtime.consoleAPICalled','Target.attachedToTarget','Target.detachedFromTarget'];
const metricNames=['TaskDuration','ScriptDuration','LayoutDuration','RecalcStyleDuration',
 'JSHeapUsedSize','JSHeapTotalSize','Nodes','Documents','JSEventListeners'];
const allowedLabel=value=>typeof value==='string'&&/^[A-Za-z0-9_.-]{1,48}$/.test(value);
const fail=()=>{throw Error('bounded qualification CDP observation unavailable');};
const tabIdentity=value=>{
 if(typeof value==='number')return Number.isSafeInteger(value)&&value>0?String(value):null;
 return typeof value==='string'&&/^[1-9][0-9]*$/.test(value)&&Number.isSafeInteger(Number(value))&&String(Number(value))===value?value:null;
};
const bytes=frame=>frame.opcode===2?atob(frame.payloadData).length:new TextEncoder().encode(frame.payloadData).length;

export async function cuaBrowserCost(tab,{label,socketUrl,tabId,connectionId,vesselId,sessionIds=[]}){
 const selectedTab=tabIdentity(tab?.id);
 if(!allowedLabel(label)||!selectedTab||tabIdentity(tabId)!==selectedTab||!connectionId||typeof vesselId!=='string'||!vesselId||sessionIds.length!==2)fail();
 const url=new URL(socketUrl);if(url.protocol!=='wss:'||url.pathname!=='/v1/vessel/browser-socket'||url.search||url.hash||url.username||url.password)fail();
 const cap=await tab.capabilities.get('cdp');
 // The capability defaults to this selected tab and current origin. Raw
 // Target.getTargetInfo is unsupported; never discover or attach ambient targets.
 await cap.send('Network.enable',{});
 await cap.send('Performance.enable',{});
 await cap.send('Runtime.enable',{});
 const seed=await cap.readEvents({limit:1,methods,timeoutMs:0});
 if(seed.truncated||!Number.isSafeInteger(seed.cursor))fail();
 let cursor=seed.cursor,closed=false,window=null,poisoned=false;const expires=Date.now()+600000;
 const sockets=new Map(),seenSockets=new Set(),serverSockets=new Set(),effects=new Set(),requests=new Map(),children=new Set();
 const totals={sent_bytes:0,received_bytes:0,sent_frames:0,received_frames:0,connections:0,closed_connections:0,authenticated_acks:0,renewal_acks:0,
  duplicate_effects:0,unknown_effects:0,refused_effects:0};
 let observedTarget=null,currentSocket=null;
 const sourceMatches=source=>{
  if(!source||tabIdentity(source.tabId)!==selectedTab)return false;
  if(source.targetId!==undefined){
   if(typeof source.targetId!=='string'||!source.targetId)return false;
   if(observedTarget!==null&&source.targetId!==observedTarget)return false;
   observedTarget=source.targetId;
  }
  return true;
 };
 const recordCommand=(text,socket)=>{
  if(!text.includes('"command"'))return;
  const frame=JSON.parse(text),command=frame.request?.command,op=command?.operation;
  if(frame.type!=='command'||command?.op!=='host_browser'||!sessionIds.includes(command.session_id)||!op?.command_id||['status','mirror','receipt'].includes(op.action))return;
  if(effects.has(op.command_id))totals.duplicate_effects++;
  effects.add(op.command_id);requests.set(frame.request_id,{commandId:op.command_id,socket});
  if(effects.size>256||requests.size>64)fail();
 };
 const recordReply=(text,socket)=>{
  if(!text.includes('"reply"'))return;
  const frame=JSON.parse(text),pending=requests.get(frame.request_id);if(frame.type!=='reply'||!pending)return;
  if(pending.socket!==socket)fail();const id=pending.commandId;
  requests.delete(frame.request_id);const outer=frame.response,inner=outer?.result?.result;
  if(outer?.outcome_unknown!==false||inner?.outcome_unknown===true)totals.unknown_effects++;
  else if(outer?.error!=null||inner?.error!=null)totals.refused_effects++;
  // A verified non-dispatch may repeat its exact ID with newly observed fences.
  if(inner?.status==='prepared'&&inner.not_dispatched===true)effects.delete(id);
 };
 async function readEvents(){
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
      const candidates=[...sockets.entries()].filter(([,s])=>s.hello&&!s.ack);
      if(candidates.length!==1||!totals.authenticated_acks&&metadata.event!=='connected'||totals.authenticated_acks&&metadata.event!=='renewal_ack')fail();
      const [id,socket]=candidates[0];
      if(currentSocket&&currentSocket!==id){const old=sockets.get(currentSocket);if(!old?.ack)fail();old.drainingAt=Date.now();}
      socket.ack=true;currentSocket=id;
      totals.authenticated_acks++;if(metadata.event==='renewal_ack')totals.renewal_acks++;
     }
     continue;
    }
    if(event.method==='Network.webSocketCreated'){
     if(p.url===socketUrl){
      if(typeof p.requestId!=='string'||!p.requestId||seenSockets.has(p.requestId)||sockets.size>=2||seenSockets.size>=64)fail();
      if(sockets.size&&(!currentSocket||!sockets.get(currentSocket)?.ack))fail();
      seenSockets.add(p.requestId);sockets.set(p.requestId,{createdAt:Date.now(),auth:false,hello:false,ack:false,drainingAt:null});totals.connections++;
     }
     continue;
    }
    if(event.method==='Network.webSocketClosed'){
     const socket=sockets.get(p.requestId);if(socket){
      if(!socket.ack)fail();
      if(p.requestId===currentSocket){
       const replacements=[...sockets.entries()].filter(([id,s])=>id!==p.requestId&&s.hello&&!s.ack);
       if(replacements.length!==1)fail();currentSocket=null;
      }else if(socket.drainingAt===null)fail();
      if([...requests.values()].some(pending=>pending.socket===p.requestId))fail();
      sockets.delete(p.requestId);totals.closed_connections++;
     }continue;
    }
    if(event.method==='Network.webSocketFrameError'){if(sockets.has(p.requestId))fail();continue;}
    if(!sockets.has(p.requestId))continue;
    const frame=p.response;if(!frame||typeof frame.payloadData!=='string'||frame.payloadData.length>4*1024*1024)fail();
    // Application frames only: control frames and TLS/HTTP overhead are separate.
    if(![1,2].includes(frame.opcode))continue;
    const sent=event.method==='Network.webSocketFrameSent';
    totals[sent?'sent_bytes':'received_bytes']+=bytes(frame);totals[sent?'sent_frames':'received_frames']++;
    const socket=sockets.get(p.requestId);
    if(!socket.hello){
     if(frame.opcode!==1)fail();const message=JSON.parse(frame.payloadData);
     if(sent){if(socket.auth||message.type!=='authenticate'||typeof message.token!=='string'||!/^[a-f0-9]{64}$/i.test(message.token))fail();socket.auth=true;}
     else{if(!socket.auth||message.type!=='hello'||message.protocol!==1||message.vessel_id!==vesselId||typeof message.socket_id!=='string'||!message.socket_id||serverSockets.has(message.socket_id))fail();socket.hello=true;serverSockets.add(message.socket_id);}
     continue;
    }
    if(frame.opcode===1){if(sent)recordCommand(frame.payloadData,p.requestId);else recordReply(frame.payloadData,p.requestId);}
   }
   cursor=result.cursor;if(!result.hasMore)return;
  }
  fail();
 }
 async function poll(){if(poisoned)fail();try{return await readEvents();}catch(error){poisoned=true;throw error;}}
 function checkTransportState(){
  if(poisoned||closed||Date.now()>=expires)fail();
  const active=sockets.get(currentSocket);if(!active?.ack||!active.hello)fail();
  let pending=0,draining=0;
  for(const [id,socket] of sockets){
   if(id===currentSocket)continue;
   if(socket.ack){if(socket.drainingAt===null||Date.now()-socket.drainingAt>30000)fail();draining++;}
   else{if(!socket.auth||Date.now()-socket.createdAt>10000)fail();pending++;}
  }
  if(pending+draining>1)fail();
  return {authenticated_open_sockets:1+draining,draining_authenticated_sockets:draining,pending_handshakes:pending,all_completed_connections_authenticated:totals.authenticated_acks+pending===totals.connections};
 }
 function transportState(){try{return checkTransportState();}catch(error){poisoned=true;throw error;}}
 async function metrics(){
  const response=await cap.send('Performance.getMetrics',{});
  const all=Object.fromEntries(response.metrics.map(m=>[m.name,m.value]));
  if(metricNames.some(name=>!Number.isFinite(all[name])))fail();
  return Object.fromEntries(metricNames.map(name=>[name,all[name]]));
 }
 const observer={
  poll,
  transportState,
  async begin(){
   await poll();const transport=transportState();if(!transport.all_completed_connections_authenticated||window)fail();
   window={at:Date.now(),metrics:await metrics(),traffic:{...totals},transport};
  },
  async finish(){
   if(!window)fail();await poll();const transport=transportState();if(!transport.all_completed_connections_authenticated)fail();
   const after=await metrics(),before=window;window=null;
   if(after.TaskDuration<before.metrics.TaskDuration)fail();
   return {transport_at_start:before.transport,transport_at_end:transport,label,status:totals.duplicate_effects||totals.unknown_effects||totals.refused_effects||requests.size?'unknown':'observed',scope:'actual CUA qualification Web tab renderer and public WSS application payload',
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
