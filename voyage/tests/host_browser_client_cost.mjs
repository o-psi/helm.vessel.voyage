// Passive metrics for an already authorized, qualification-owned Chromium page.
// No navigation, login, effects, payload recording or clock manipulation.
import {performance} from 'node:perf_hooks';

const metricNames=['TaskDuration','ScriptDuration','LayoutDuration','RecalcStyleDuration',
 'JSHeapUsedSize','JSHeapTotalSize','Nodes','Documents','JSEventListeners'];
export async function browserCost(context,page,{webSocketUrls=[],nativeOperationUrls=[],maxMilliseconds=60000}={}){
 if(webSocketUrls.length>8||nativeOperationUrls.length>8||maxMilliseconds<1000||maxMilliseconds>60000)throw Error('measurement scope exceeds bound');
 const selected=new Set(webSocketUrls),httpSelected=new Set(nativeOperationUrls),sockets=new Set(),requests=new Set();
 const traffic={ws_sent_payload_bytes:0,ws_received_payload_bytes:0,ws_sent_frames:0,ws_received_frames:0,
  selected_ws_connections_seen:0,http_operation_response_encoded_bytes:0,http_operation_responses:0};
 const cdp=await context.newCDPSession(page),handlers=[];let overflow=false;
 const on=(name,handler)=>{cdp.on(name,handler);handlers.push([name,handler]);};
 on('Network.webSocketCreated',event=>{if(selected.has(event.url)){if(sockets.size>=64){overflow=true;return;}sockets.add(event.requestId);traffic.selected_ws_connections_seen++;}});
 on('Network.webSocketClosed',event=>sockets.delete(event.requestId));
 for(const [name,direction] of [['Network.webSocketFrameSent','sent'],['Network.webSocketFrameReceived','received']]){
  on(name,event=>{
   if(!sockets.has(event.requestId))return;
   // Frame contents (including auth/private input) are transient only. Retain
   // byte totals, never request IDs, messages, URLs, cookies or headers.
   const frame=event.response,bytes=frame.opcode===2?Buffer.from(frame.payloadData,'base64').length:Buffer.byteLength(frame.payloadData,'utf8');
   traffic[`ws_${direction}_payload_bytes`]+=bytes;traffic[`ws_${direction}_frames`]++;
  });
 }
 on('Network.requestWillBeSent',event=>{if(httpSelected.has(event.request.url)){if(requests.size>=128){overflow=true;return;}requests.add(event.requestId);}});
 on('Network.loadingFailed',event=>requests.delete(event.requestId));
 on('Network.loadingFinished',event=>{if(requests.delete(event.requestId)){traffic.http_operation_response_encoded_bytes+=event.encodedDataLength;traffic.http_operation_responses++;}});
 try{await cdp.send('Performance.enable');await cdp.send('Network.enable');}
 catch(error){for(const [name,handler] of handlers)cdp.off(name,handler);await cdp.detach();throw error;}
 const read=async()=>{
  const result=await cdp.send('Performance.getMetrics'),all=Object.fromEntries(result.metrics.map(m=>[m.name,m.value]));
  return Object.fromEntries(metricNames.map(name=>[name,Number.isFinite(all[name])?all[name]:null]));
 };
 let before;
 try{before=await read();}catch(error){for(const [name,handler] of handlers)cdp.off(name,handler);await cdp.detach();throw error;}
 const started=performance.now();let finishing=null,timer;
 const finish=()=>finishing??=(async()=>{
   clearTimeout(timer);
   try{return {elapsed_ms:performance.now()-started,before,after:await read(),traffic:{...traffic},
    status:overflow?'scope_overflow':'observed',websocket_traffic_qualified:selected.size>0&&traffic.selected_ws_connections_seen>0,
    cpu_scope:'renderer task time; whole-browser OS CPU/RSS require the owned PID ledger',
    traffic_scope:'selected WebSocket application payload and CDP native HTTP response bytes; excludes TLS/wire overhead and native Helm WSS bytes'};}
   finally{for(const [name,handler] of handlers)cdp.off(name,handler);await cdp.detach();}
  })();
 timer=setTimeout(()=>{void finish().catch(()=>{});},maxMilliseconds);
 return {async snapshot(){if(finishing)throw Error('measurement finished; read stop result');return {elapsed_ms:performance.now()-started,renderer:await read(),traffic:{...traffic}};},stop:finish};
}

export async function inputVisibleLatency(action,visible){
 const started=performance.now();await action();await visible();
 return {input_to_visible_ms:performance.now()-started,scope:'single explicit input and observed replay marker; no retries'};
}
